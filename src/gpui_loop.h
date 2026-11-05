// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

#pragma once

#include <atomic>
#include <chrono>
#include <cstdint>
#include <functional>
#include <mutex>
#include <thread>
#include <vector>

namespace gpui {
class SocketServer;

/// Repeating callback owned by the host loop. Start and Stop belong to the loop
/// thread; Tick runs on it. A timer never outlives the loop it registered with.
class Timer {
public:
	std::function<void()> on_tick;

	/// Starts ticking every `interval_ms`, beginning one interval from now.
	void Start(int interval_ms);
	void Stop();
	bool Running() const { return running; }

private:
	friend class EventLoop;
	bool running = false;
	int interval_ms = 0;
	std::chrono::steady_clock::time_point due;
};

/// The host's single thread. Everything that must happen on it goes through
/// here: posted tasks, timers, and the bridge socket's poll.
class EventLoop {
public:
	static EventLoop &Current();

	/// Whether the caller runs on the loop thread. Work that must not block the
	/// bridge (dialogs, progress cards) pumps the loop instead of waiting.
	bool OnLoopThread() const;

	/// Hands `task` to the loop thread and interrupts a blocked poll. Safe from
	/// any thread.
	void Post(std::function<void()> task);

	/// `server` is stepped from Run() until Stop(); it also supplies the wakeup
	/// that Post() uses, so it must stay alive for the whole run.
	void Attach(SocketServer *server);

	/// Drains work until Stop() is called or the loop finds nothing left to do.
	void Run();

	/// One step of Run(): posted tasks, due timers, then up to `timeout_ms` of
	/// waiting for socket traffic. Lets a blocking caller (a progress card, a
	/// script dialog) keep the host answering while it waits.
	void Pump(int timeout_ms);
	void Stop();

private:
	friend class Timer;
	void AddTimer(Timer *timer);
	void RemoveTimer(Timer *timer);
	void DrainPosted();
	void RunDueTimers();
	bool HasPosted();
	int TimeUntilNextTimer() const;

	SocketServer *server = nullptr;
	std::atomic<std::thread::id> loop_thread {};
	std::vector<Timer *> timers;
	std::vector<std::function<void()>> posted;
	std::mutex mutex;
	bool running = false;
};
}
