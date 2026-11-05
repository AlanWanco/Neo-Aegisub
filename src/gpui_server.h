// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

#pragma once

#include <cstddef>
#include <map>
#include <string>
#include <string_view>

namespace gpui {
/// Local TCP listener driven by explicit Step() calls. The host owns its main
/// loop, so this class spawns no threads and touches no toolkit event queue:
/// every callback runs on the thread that called Step().
class SocketServer {
public:
	/// Large enough for a snapshot with a full subtitle file and for a frame
	/// header; anything beyond it is answered with an error and dropped.
	static constexpr size_t max_request_size = 2 * 1024 * 1024;

	struct Handler {
		/// Runs once per accepted connection, before any request.
		virtual void OnConnect(int client) { }
		/// A complete request line arrived. The request buffer is reused after
		/// the call returns, so copy anything kept. Answer with Send().
		virtual void OnRequest(int client, std::string const& request) = 0;
		/// Runs after the connection is gone: EOF, socket error, or CloseClient.
		virtual void OnDisconnect(int) { }
		virtual ~Handler() = default;
	};

	explicit SocketServer(Handler *handler);
	~SocketServer();
	SocketServer(SocketServer const&) = delete;
	SocketServer& operator=(SocketServer const&) = delete;

	/// Binds 127.0.0.1 on an ephemeral port. `port` receives the chosen port.
	/// Returns false when the socket cannot be created; nothing is left open.
	/// Also creates the self-pipe that Wake() writes to.
	bool Listen(unsigned short &port);
	void Close();
	bool Running() const { return listener >= 0; }

	/// One non-blocking pass: accept, read, write. Waits at most `timeout_ms`
	/// for activity and returns as soon as there is work to do.
	void Step(int timeout_ms);

	/// Makes a blocked Step() return. Safe to call from any thread; this is how
	/// work posted by a worker gets back to the loop that owns the server.
	void Wake();

	/// Queues `head`, a newline, then `body` verbatim for client `id` and
	/// flushes what the socket accepts. A second call replaces the queue only
	/// after the first response is fully written, so callers must send one
	/// response per request.
	void Send(int client, std::string head, std::string_view body = {});
	size_t Pending(int client) const;
	void CloseClient(int client);

private:
	struct Connection {
		int fd = -1;
		std::string in;
		std::string out;
		size_t out_offset = 0;
		/// A response is queued: stop parsing requests until it is flushed.
		bool busy = false;
		/// Close the connection as soon as the queued reply is fully written.
		bool close_after = false;
	};

	void Accept();
	void Receive(Connection &conn);
	void Dispatch(Connection &conn);
	/// Returns false when the connection must be dropped.
	bool Flush(Connection &conn);

	Handler *handler;
	int listener = -1;
	std::map<int, Connection> connections;
	bool sockets_ready = false;
	/// Self-pipe cast to a platform socket: the header stays free of winsock.
	void *waker_read = nullptr;
	void *waker_write = nullptr;
	void DrainWake();
};
}
