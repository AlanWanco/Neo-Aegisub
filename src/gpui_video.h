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

    /// Answers {"op":"video_frame", "since":N?, "variant":"display"?, "format":"bgra8"?}.
    /// Display replies default to tightly packed BGRA8, top row first; `format:"png"`
    /// encodes the displayed frame on demand. The raw/subs variants always return PNG.
    /// The head reports `serial`, `frame` (-1 when none), `width`, `height`,
    /// `pixel_format` and `byte_length`. Only BGRA8 previews use `since` to return
    /// an empty body with `unchanged:true`; no frame also returns an empty body.
    RawReply Frame(json::Object const& request);

    /// Stops listening for frames and drops the held one. Safe to repeat.
    void Close();
};
}
