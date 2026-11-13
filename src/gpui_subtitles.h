// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

#pragma once

#include "gpui_audio.h"

#include <libaegisub/cajun/elements.h>

#include <string>

namespace agi { struct Context; }

namespace gpui {
/// Subtitles-only bitmap for the native overlay drawn above mpv.
///
/// mpv renders subtitles with its own libass, so a CSRI renderer (VSFilter,
/// VSFilterMod) can only be shown while playback runs by drawing the host's
/// rendering in a second window over mpv's picture. This answers
/// {"op":"subtitle_layer","since":N?}: the head reports `serial`, `empty`,
/// `x`, `y`, `width`, `height`, `animated`, `next_change` and `byte_length`,
/// the body is premultiplied RGBA8, top row first, cropped to the subtitles.
///
/// A bitmap is rasterised only when the visible lines change or while one of
/// them animates; `next_change` tells the front end when the line set may
/// change next, so a static sign costs one raster per line.
RawReply SubtitleLayer(agi::Context *context, json::Object const& request);
}
