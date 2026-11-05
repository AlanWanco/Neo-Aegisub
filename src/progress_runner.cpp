// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

#include "progress_runner.h"

#include "gpui_bridge.h"
#include "gpui_loop.h"

#include <libaegisub/dispatch.h>
#include <libaegisub/exception.h>
#include <libaegisub/util_osx.h>

#include <algorithm>
#include <atomic>
#include <chrono>
#include <exception>
#include <thread>

namespace {
/// Publishes progress to the GPUI front end, which draws the card itself.
class GpuiProgressSink final : public agi::ProgressSink {
	std::string title = "Working";
	std::string message;
	double fraction = 0.;
	bool cancelled = false;

	void Publish() {
		gpui::PushProgress(title, message, fraction, true);
	}

public:
	void SetIndeterminate() override {
		fraction = -1.;
		Publish();
	}

	void SetTitle(std::string const& value) override {
		title = value;
		Publish();
	}

	void SetMessage(std::string const& value) override {
		message = value;
		Publish();
	}

	void SetProgress(int64_t cur, int64_t max) override {
		fraction = max > 0 ? std::clamp(double(cur) / double(max), 0., 1.) : 0.;
		Publish();
	}

	void Log(std::string const& str) override {
		gpui::PushNotice(title, str);
	}

	bool IsCancelled() override {
		cancelled = cancelled || gpui::ProgressCancelled();
		return cancelled;
	}
};
}

void ProgressRunner::Run(std::function<void(agi::ProgressSink *)> task) {
	GpuiProgressSink ps;
	std::atomic<bool> done{false};
	std::exception_ptr failure;

	auto work = [&] {
		agi::osx::AppNapDisabler app_nap_disabler("Working");
		try {
			task(&ps);
		}
		catch (...) {
			failure = std::current_exception();
		}
		done = true;
	};

	if (gpui::EventLoop::Current().OnLoopThread()) {
		agi::dispatch::Background().Async(work);
		// The front end keeps drawing the card while the bridge answers, so the
		// loop has to keep turning until the work finishes.
		while (!done) gpui::EventLoop::Current().Pump(8);
	}
	else {
		work();
	}

	bool const cancelled = ps.IsCancelled();
	gpui::ClearProgress();
	if (failure) std::rethrow_exception(failure);
	if (cancelled) throw agi::UserCancelException("Cancelled by user");
}
