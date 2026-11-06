// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

#include "gpui_loop.h"

#include "gpui_server.h"

#include <libaegisub/log.h>

#include <algorithm>
#include <thread>

namespace gpui {
namespace {
using clock_t_ = std::chrono::steady_clock;

int ClampToMilliseconds(clock_t_::duration value) {
	auto const ms = std::chrono::duration_cast<std::chrono::milliseconds>(value).count();
	if (ms < 0) return 0;
	if (ms > 1'000'000) return 1'000'000;
	return static_cast<int>(ms);
}
}

void Timer::Start(int interval) {
	if (interval <= 0) return;
	interval_ms = interval;
	running = true;
	due = clock_t_::now() + std::chrono::milliseconds(interval);
	EventLoop::Current().AddTimer(this);
}

void Timer::Stop() {
	if (!running) return;
	running = false;
	EventLoop::Current().RemoveTimer(this);
}

EventLoop &EventLoop::Current() {
	static EventLoop loop;
	return loop;
}


bool EventLoop::OnLoopThread() const {
	return loop_thread == std::this_thread::get_id();
}

void EventLoop::Post(std::function<void()> task) {
	if (!task) return;
	{
		std::lock_guard<std::mutex> lock(mutex);
		posted.push_back(std::move(task));
	}
	if (server) server->Wake();
}

void EventLoop::Attach(SocketServer *socket_server) {
	server = socket_server;
}

void EventLoop::AddTimer(Timer *timer) {
	if (timer && std::find(timers.begin(), timers.end(), timer) == timers.end())
		timers.push_back(timer);
}

void EventLoop::RemoveTimer(Timer *timer) {
	timers.erase(std::remove(timers.begin(), timers.end(), timer), timers.end());
}

void EventLoop::DrainPosted() {
	std::vector<std::function<void()>> tasks;
	{
		std::lock_guard<std::mutex> lock(mutex);
		tasks.swap(posted);
	}
	for (auto &task : tasks) {
		try {
			task();
		}
		catch (std::exception const& err) {
			LOG_E("host/loop") << "Task raised: " << err.what();
		}
		catch (...) {
			LOG_E("host/loop") << "Task raised an unknown exception";
		}
	}
}

void EventLoop::RunDueTimers() {
	if (timers.empty()) return;
	auto const now = clock_t_::now();
	// Snapshot: a callback may start or stop timers, including its own.
	std::vector<Timer *> const snapshot = timers;
	for (Timer *timer : snapshot) {
		if (!timer->running) continue;
		if (timer->due > now) continue;
		if (std::find(timers.begin(), timers.end(), timer) == timers.end()) continue;
		timer->due = now + std::chrono::milliseconds(timer->interval_ms);
		if (timer->on_tick) timer->on_tick();
	}
}

int EventLoop::TimeUntilNextTimer() const {
	int best = -1;
	auto const now = clock_t_::now();
	for (Timer *timer : timers) {
		if (!timer->running) continue;
		int const wait = ClampToMilliseconds(timer->due - now);
		if (best < 0 || wait < best) best = wait;
	}
	return best;
}

bool EventLoop::HasPosted() {
	std::lock_guard<std::mutex> lock(mutex);
	return !posted.empty();
}

void EventLoop::Pump(int timeout) {
	loop_thread = std::this_thread::get_id();
	DrainPosted();
	RunDueTimers();
	if (server) server->Step(timeout);
	else if (timeout > 0) std::this_thread::sleep_for(std::chrono::milliseconds(timeout));
}

void EventLoop::Run() {
	loop_thread = std::this_thread::get_id();
	running = true;
	while (running) {
		DrainPosted();
		RunDueTimers();
		if (!running) break;

		int const timeout = HasPosted() ? 0 : TimeUntilNextTimer();
		if (server) {
			server->Step(timeout);
		}
		else if (timeout > 0) std::this_thread::sleep_for(std::chrono::milliseconds(timeout));
		else if (timeout < 0) break;
	}
	running = false;
}

void EventLoop::Stop() {
	running = false;
	if (server) server->Wake();
}
}
