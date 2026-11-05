// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

#pragma once

#include <memory>
#include <string>

struct VideoFrame;

/// Receives frames and errors from AsyncVideoProvider. Every method runs on the
/// main thread: the provider posts its results there before calling in, so the
/// implementation never needs its own locking.
class AsyncVideoSink {
public:
	/// A requested frame finished decoding and rendering.
	virtual void FrameReady(std::shared_ptr<VideoFrame> frame, double time, int frame_number) = 0;
	/// The video provider failed.
	virtual void VideoError(std::string message) = 0;
	/// The subtitles provider failed to initialize.
	virtual void SubtitlesError(std::string message) = 0;

protected:
	~AsyncVideoSink() = default;
};
