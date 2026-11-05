// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

#pragma once

#include <libaegisub/background_runner.h>

/// Runs a cancellable background task while the front end draws its own
/// progress card. The host used to open a wx dialog here; the front end owns
/// that surface now, and this class only publishes the sink's state to it.
class ProgressRunner final : public agi::BackgroundRunner {
public:
	/// Runs `task` on the background queue and blocks the calling thread until
	/// it finishes, pumping the host loop so the bridge keeps answering.
	/// Throws agi::UserCancelException if the user cancelled.
	void Run(std::function<void(agi::ProgressSink *)> task) override;
};
