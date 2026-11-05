// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

#pragma once

#include <libaegisub/cajun/elements.h>

#include <memory>
#include <string>

namespace agi { struct Context; }
namespace gpui {
std::string VideoSettings(agi::Context *context, json::Object const& request);

/// Native OpenGL window showing only the video, driven by the GPUI front end.
///
/// The window is a top-level wxFrame whose only child is the VideoDisplay canvas: no menus and
/// no toolbars. While it is open, agi::Context::videoDisplay points at it. Closing it, unloading
/// the video or calling Close() restores the previous display.
class VideoSurface final {
    struct Impl;
    std::unique_ptr<Impl> impl;

public:
    explicit VideoSurface(agi::Context *context);
    /// Hides and destroys the window immediately and restores the previous context->videoDisplay.
    /// The context must still be alive.
    ~VideoSurface();
    VideoSurface(VideoSurface const&) = delete;
    VideoSurface& operator=(VideoSurface const&) = delete;

    /// Answers one {"op":"video_surface", ...} request and returns the JSON reply. Actions:
    ///   "show", "hide", "state"
    ///   "configure": optional "tool" (cross, drag, rotate/z, rotate/xy, scale, clip, vector_clip),
    ///                "subtool" (vector_clip only, 0 to VCLIP_LAST-1) and "zoom" (0.125 to 10)
    ///   "command":   "name" is a video/tool/* command, including video/tool/vclip/*. Opens the
    ///                window if needed, runs the command and reports the tool actually active.
    /// Throws std::invalid_argument for a rejected request.
    std::string Handle(json::Object const& request);
    bool IsVisible() const;

    /// Hides and destroys the window immediately and restores the previous context->videoDisplay.
    void Close();
};
}
