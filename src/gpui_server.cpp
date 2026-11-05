// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

#include "gpui_server.h"

#include <vector>

#ifdef _WIN32
#include <winsock2.h>
#include <ws2tcpip.h>
#else
#include <cerrno>
#include <fcntl.h>
#include <netinet/in.h>
#include <poll.h>
#include <sys/socket.h>
#include <unistd.h>
#endif

namespace gpui {
namespace {
#ifdef _WIN32
using socket_t = SOCKET;
constexpr socket_t invalid_fd = INVALID_SOCKET;
using pollfd_t = WSAPOLLFD;
#else
using socket_t = int;
constexpr socket_t invalid_fd = -1;
using pollfd_t = pollfd;
#endif

bool would_block() {
#ifdef _WIN32
	int const error = WSAGetLastError();
	return error == WSAEWOULDBLOCK || error == WSAEINTR;
#else
	return errno == EAGAIN || errno == EWOULDBLOCK || errno == EINTR;
#endif
}

void close_socket(socket_t socket) {
#ifdef _WIN32
	closesocket(socket);
#else
	::close(socket);
#endif
}

bool set_nonblocking(socket_t socket) {
#ifdef _WIN32
	u_long on = 1;
	return ioctlsocket(socket, FIONBIO, &on) == 0;
#else
	int const flags = fcntl(socket, F_GETFL, 0);
	return flags != -1 && fcntl(socket, F_SETFL, flags | O_NONBLOCK) != -1;
#endif
}

bool send_bytes(socket_t socket, char const *data, size_t size, size_t &written) {
#ifdef MSG_NOSIGNAL
	int const flags = MSG_NOSIGNAL;
#else
	int const flags = 0;
#endif
#ifdef _WIN32
	auto const sent = ::send(socket, data, static_cast<int>(size), flags);
#else
	auto const sent = ::send(socket, data, size, flags);
#endif
	if (sent >= 0) {
		written = static_cast<size_t>(sent);
		return true;
	}
	written = 0;
	return would_block();
}

#ifdef _WIN32
bool sockets_started = false;
#endif
}

SocketServer::SocketServer(Handler *handler) : handler(handler) { }

SocketServer::~SocketServer() { Close(); }

bool SocketServer::Listen(unsigned short &port) {
	Close();
#ifdef _WIN32
	if (!sockets_started) {
		WSADATA data{};
		if (WSAStartup(MAKEWORD(2, 2), &data) != 0) return false;
		sockets_started = true;
	}
#endif
	sockets_ready = true;
	socket_t const socket = ::socket(AF_INET, SOCK_STREAM, 0);
	if (socket == invalid_fd) return false;

	int reuse = 1;
	setsockopt(socket, SOL_SOCKET, SO_REUSEADDR, reinterpret_cast<char const *>(&reuse), sizeof reuse);
#ifndef _WIN32
#ifdef SO_NOSIGPIPE
	int const no_signal = 1;
	setsockopt(socket, SOL_SOCKET, SO_NOSIGPIPE, &no_signal, sizeof no_signal);
#endif
#endif

	sockaddr_in address{};
	address.sin_family = AF_INET;
	address.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
	address.sin_port = 0;
	bool const bound = ::bind(socket, reinterpret_cast<sockaddr *>(&address), sizeof address) == 0;
	if (!bound || ::listen(socket, 8) != 0 || !set_nonblocking(socket)) {
		close_socket(socket);
		return false;
	}

	sockaddr_in local{};
#ifdef _WIN32
	int local_size = sizeof local;
#else
	socklen_t local_size = sizeof local;
#endif
	if (getsockname(socket, reinterpret_cast<sockaddr *>(&local), &local_size) != 0) {
		close_socket(socket);
		return false;
	}
	port = ntohs(local.sin_port);
	listener = static_cast<int>(socket);

	// Self-pipe: Wake() writes one byte, Step() drains it and returns, so a
	// worker thread can interrupt the poll without touching the loop's state.
#ifdef _WIN32
	SOCKET waker_listener = ::socket(AF_INET, SOCK_STREAM, 0);
	sockaddr_in waker_address = local;
	waker_address.sin_port = 0;
	bool waker_ready = waker_listener != INVALID_SOCKET
		&& ::bind(waker_listener, reinterpret_cast<sockaddr *>(&waker_address), sizeof waker_address) == 0
		&& ::listen(waker_listener, 1) == 0;
	int waker_size = sizeof waker_address;
	if (waker_ready)
		waker_ready = getsockname(waker_listener, reinterpret_cast<sockaddr *>(&waker_address), &waker_size) == 0;
	SOCKET waker_client = waker_ready ? ::socket(AF_INET, SOCK_STREAM, 0) : INVALID_SOCKET;
	if (waker_client != INVALID_SOCKET)
		waker_ready = ::connect(waker_client, reinterpret_cast<sockaddr *>(&waker_address), sizeof waker_address) == 0;
	SOCKET waker_server = waker_ready ? ::accept(waker_listener, nullptr, nullptr) : INVALID_SOCKET;
	close_socket(waker_listener);
	if (waker_server != INVALID_SOCKET && set_nonblocking(waker_server)) {
		waker_read = reinterpret_cast<void *>(waker_server);
		waker_write = reinterpret_cast<void *>(waker_client);
	}
	else {
		if (waker_server != INVALID_SOCKET) close_socket(waker_server);
		if (waker_client != INVALID_SOCKET) close_socket(waker_client);
	}
#else
	int waker[2] = {-1, -1};
	if (socketpair(AF_UNIX, SOCK_STREAM, 0, waker) == 0
		&& set_nonblocking(waker[0]) && set_nonblocking(waker[1])) {
		waker_read = reinterpret_cast<void *>(static_cast<intptr_t>(waker[0]));
		waker_write = reinterpret_cast<void *>(static_cast<intptr_t>(waker[1]));
	}
	else {
		if (waker[0] >= 0) close(waker[0]);
		if (waker[1] >= 0) close(waker[1]);
	}
#endif
	return port != 0;
}

void SocketServer::Close() {
	while (!connections.empty()) CloseClient(connections.begin()->first);
	if (listener >= 0) {
		close_socket(static_cast<socket_t>(listener));
		listener = -1;
	}
	for (void *handle : {waker_read, waker_write}) {
		if (handle) close_socket(static_cast<socket_t>(reinterpret_cast<intptr_t>(handle)));
	}
	waker_read = waker_write = nullptr;
}

void SocketServer::Accept() {
	for (;;) {
		sockaddr_in remote{};
#ifdef _WIN32
		int remote_size = sizeof remote;
#else
		socklen_t remote_size = sizeof remote;
#endif
		socket_t const socket = ::accept(static_cast<socket_t>(listener), reinterpret_cast<sockaddr *>(&remote), &remote_size);
		if (socket == invalid_fd) return;
		if (!set_nonblocking(socket)) {
			close_socket(socket);
			continue;
		}
		Connection connection;
		connection.fd = static_cast<int>(socket);
		int const client = connection.fd;
		connections.emplace(client, std::move(connection));
		if (handler) handler->OnConnect(client);
	}
}

void SocketServer::Receive(Connection &conn) {
	char buffer[65536];
	for (;;) {
		int const client = conn.fd;
		auto const got = ::recv(static_cast<socket_t>(client), buffer, sizeof buffer, 0);
		if (got == 0) {
			CloseClient(client);
			return;
		}
		if (got < 0) {
			if (!would_block()) CloseClient(client);
			return;
		}
		conn.in.append(buffer, static_cast<size_t>(got));
		if (conn.in.size() > max_request_size) {
			conn.in.clear();
			Send(client, R"({"ok":false,"error":"request too large"})");
			return;
		}
		Dispatch(conn);
		if (connections.find(client) == connections.end()) return;
		if (conn.busy) return;
	}
}

void SocketServer::Dispatch(Connection &conn) {
	for (;;) {
		auto const newline = conn.in.find('\n');
		if (newline == std::string::npos) return;
		std::string request = conn.in.substr(0, newline);
		conn.in.erase(0, newline + 1);
		int const client = conn.fd;
		handler->OnRequest(client, request);
		if (connections.find(client) == connections.end()) return;
		if (conn.busy) return;
	}
}

void SocketServer::Send(int client, std::string head, std::string_view body) {
	auto it = connections.find(client);
	if (it == connections.end()) return;
	Connection &conn = it->second;
	conn.out.append(head);
	conn.out.push_back('\n');
	conn.out.append(body);
	conn.busy = true;
	// One request gets one reply on its own connection: the reply ends with a
	// newline and the socket is closed, which is what the frontend waits for.
	conn.close_after = true;
	Flush(conn);
}

size_t SocketServer::Pending(int client) const {
	auto it = connections.find(client);
	return it == connections.end() ? 0 : it->second.out.size() - it->second.out_offset;
}

bool SocketServer::Flush(Connection &conn) {
	int const client = conn.fd;
	while (conn.out_offset < conn.out.size()) {
		size_t written = 0;
		if (!send_bytes(static_cast<socket_t>(client),
				conn.out.data() + conn.out_offset, conn.out.size() - conn.out_offset, written)) {
			CloseClient(client);
			return false;
		}
		if (!written) return true;
		conn.out_offset += written;
	}
	conn.out.clear();
	conn.out_offset = 0;
	conn.busy = false;
	if (conn.close_after) CloseClient(client);
	return true;
}

void SocketServer::CloseClient(int client) {
	auto it = connections.find(client);
	if (it == connections.end()) return;
	close_socket(static_cast<socket_t>(it->second.fd));
	connections.erase(it);
	if (handler) handler->OnDisconnect(client);
}

void SocketServer::Wake() {
	if (!waker_write) return;
	char const byte = 1;
	::send(static_cast<socket_t>(reinterpret_cast<intptr_t>(waker_write)), &byte, 1, 0);
}

void SocketServer::DrainWake() {
	if (!waker_read) return;
	char buffer[64];
	auto const socket = static_cast<socket_t>(reinterpret_cast<intptr_t>(waker_read));
	while (::recv(socket, buffer, sizeof buffer, 0) > 0) { }
}

void SocketServer::Step(int timeout_ms) {
	if (listener < 0) return;

	// A pipelined request can still be buffered after a response is flushed.
	for (auto &entry : connections) {
		if (!entry.second.busy && !entry.second.in.empty()) {
			auto &conn = entry.second;
			Dispatch(conn);
		}
	}

	std::vector<pollfd_t> fds;
	std::vector<int> order;
	size_t const first_client = 1 + (waker_read ? 1 : 0);
	fds.push_back(pollfd_t{static_cast<socket_t>(listener), POLLIN, 0});
	if (waker_read)
		fds.push_back(pollfd_t{static_cast<socket_t>(reinterpret_cast<intptr_t>(waker_read)), POLLIN, 0});
	for (auto const& [id, conn] : connections) {
		bool const writing = conn.out_offset < conn.out.size();
		pollfd_t entry{static_cast<socket_t>(conn.fd), 0, 0};
		if (!conn.busy) entry.events |= POLLIN;
		if (writing) entry.events |= POLLOUT;
		fds.push_back(entry);
		order.push_back(id);
	}

#ifdef _WIN32
	int const active = WSAPoll(fds.data(), static_cast<ULONG>(fds.size()), timeout_ms);
#else
	int const active = ::poll(fds.data(), static_cast<nfds_t>(fds.size()), timeout_ms);
	if (active < 0 && errno == EINTR) return;
#endif
	if (active <= 0) return;

	if (fds[0].revents & POLLIN) Accept();
	if (waker_read && fds[1].revents) DrainWake();

	for (size_t i = 0; i < order.size(); ++i) {
		auto it = connections.find(order[i]);
		if (it == connections.end()) continue;
		Connection &conn = it->second;
		short const revents = fds[i + first_client].revents;
		if (!revents) continue;
		if (revents & (POLLERR | POLLNVAL)) {
			CloseClient(conn.fd);
			continue;
		}
		if (revents & POLLOUT) {
			if (!Flush(conn)) continue;
		}
		if ((revents & (POLLIN | POLLHUP)) && !conn.busy) Receive(conn);
	}
}
}
