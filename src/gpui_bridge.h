// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

#pragma once

#include <functional>
#include <memory>
#include <string>
#include <vector>

namespace agi { struct Context; }

namespace gpui {
class SocketServer;

/// Whether a GPUI front end is connected, i.e. whether warnings should be sent
/// to it instead of opening wx dialogs the front end never shows.
bool FrontendAttached();

/// Queue a warning for the GPUI front end. The next document snapshot delivers
/// it exactly once; with no front end attached the notice is dropped.
void PushNotice(std::string title, std::string body);
/// Same, but the front end offers a "do not show again" switch that writes
/// `suppress_option` (set to 0, the enum's "ignore") when it is ticked.
void PushNotice(std::string title, std::string body, std::string suppress_option);
/// Notice that carries an action row; `kind` names which one the front end draws.
void PushNotice(std::string title, std::string body, std::string suppress_option, std::string kind);
/// Question whose answers are buttons. `apply` receives the chosen index once
/// the front end answers, so a prompt can stay asynchronous instead of blocking
/// the host thread the bridge is waiting on.
void PushChoice(std::string title, std::string body, std::string suppress_option,
                std::string kind, std::vector<std::string> choices,
                std::function<void(int)> apply);
/// Runs a command on the front end instead of opening the host's own dialog.
/// Commands the front end owns (the colour pickers today) use this so Lua and
/// the legacy menus do not pop a wx window over the GPUI frame.
void PushCommand(std::string name);

/// Ask the front end a question and wait for the answer, pumping the host loop
/// so the bridge keeps answering while the prompt is on screen. Returns -1 when
/// no front end is attached or the prompt was dismissed.
int AskChoice(std::string title, std::string body, std::string suppress_option,
              std::string kind, std::vector<std::string> choices);

/// Replace the front end's status line text.
void PushStatus(std::string text);

/// Publish a long-running task's progress so the front end can draw its own
/// progress card. `fraction` is 0..1, or negative for an indeterminate task.
/// The wx dialog is skipped entirely while a front end is attached.
void PushProgress(std::string title, std::string message, double fraction,
                  bool cancellable);
void ClearProgress();
/// Whether the user pressed cancel on the front end's progress card.
bool ProgressCancelled();
}

class GpuiBridge {
	struct Impl;
	std::unique_ptr<Impl> impl;

public:
	explicit GpuiBridge(agi::Context *context);
	~GpuiBridge();
	GpuiBridge(const GpuiBridge&) = delete;
	GpuiBridge& operator=(const GpuiBridge&) = delete;

	bool Start();
	void Stop();
	bool IsRunning() const;
	std::string Endpoint() const;
	std::string Token() const;

	/// The socket the host event loop steps; requests run on the loop thread.
	gpui::SocketServer &Server();
};
