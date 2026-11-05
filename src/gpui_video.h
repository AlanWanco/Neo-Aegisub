// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

#pragma once

#include "gpui_audio.h"

#include <libaegisub/cajun/elements.h>

#include <memory>
#include <string>

namespace agi { struct Context; }
namespace gpui {
std::string VideoSettings(agi::Context *context, json::Object const& request);

/// Window-less video output. Decoded, subtitle-composited frames are kept as they
/// arrive from the video provider and handed to the frontend as raw pixels; the
/// frontend draws them on its own GPU surface. The host creates no video window.
class VideoFrames final {
    struct Impl;
    std::unique_ptr<Impl> impl;

public:
    explicit VideoFrames(agi::Context *context);
    /// The context must still be alive.
    ~VideoFrames();
    VideoFrames(VideoFrames const&) = delete;
    VideoFrames& operator=(VideoFrames const&) = delete;

    /// Answers {"op":"video_frame", "since":N?}. The head reports `serial`, `frame`
    /// (-1 when none), `width`, `height`, `pixel_format` ("bgra8") and `byte_length`.
    /// The body is the newest frame, tightly packed, top row first. When `since`
    /// equals the current `serial`, or no frame exists, the body is empty and
    /// `unchanged` says whether the caller's copy is current.
    RawReply Frame(json::Object const& request);

    /// Stops listening for frames and drops the held one. Safe to repeat.
    void Close();
};
}
