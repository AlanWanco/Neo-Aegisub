// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

#include "gpui_subtitles.h"

#include "ass_dialogue.h"
#include "ass_file.h"
#include "async_video_provider.h"
#include "include/aegisub/context.h"
#include "project.h"
#include "video_controller.h"
#include "video_frame.h"

#include <libaegisub/cajun/writer.h>

#include <algorithm>
#include <cstdint>
#include <sstream>
#include <stdexcept>
#include <string_view>
#include <vector>

namespace {
/// The last bitmap handed out, so a static sign is rasterised once per line
/// instead of once per requested frame.
struct Cache {
	AsyncVideoProvider const *provider = nullptr;
	uint64_t fingerprint = 0;
	int64_t time = -1;
	bool animated = false;
	int64_t serial = 0;
	int video_width = 0;
	int video_height = 0;
	int x = 0;
	int y = 0;
	int width = 0;
	int height = 0;
	std::vector<unsigned char> rgba;
};

/// Main-thread only: the bridge answers one request at a time.
Cache cache;

struct Visible {
	uint64_t fingerprint = 1469598103934665603ull;
	bool visible = false;
	bool animated = false;
	int64_t next_change = 500;
};

/// Tags whose result depends on the frame, not only on the line text: karaoke,
/// fades, movement and anything driven by `\t`.
bool AnimatedText(std::string const &text) {
	static constexpr std::string_view const tags[] = {
		"\\t(", "\\move(", "\\fad(", "\\fade(", "\\k", "\\K",
	};
	return std::any_of(std::begin(tags), std::end(tags),
		[&](std::string_view tag) { return text.find(tag) != std::string::npos; });
}

uint64_t Mix(uint64_t hash, std::string_view bytes) {
	for (unsigned char byte : bytes) {
		hash ^= byte;
		hash *= 1099511628211ull;
	}
	return hash;
}

Visible VisibleLines(agi::Context *context, int64_t time) {
	Visible out;
	for (auto const &line : context->ass->Events) {
		if (line.Comment || line.End <= line.Start) continue;
		int64_t const start = line.Start;
		int64_t const end = line.End;
		if (start <= time && time < end) {
			std::string const &text = line.Text.get();
			out.visible = true;
			out.animated = out.animated || AnimatedText(text);
			out.fingerprint = Mix(out.fingerprint, text);
			out.fingerprint ^= static_cast<uint64_t>(start) * 1000003ull ^ static_cast<uint64_t>(end);
			out.next_change = std::min(out.next_change, end - time);
		}
		else if (start > time) {
			out.next_change = std::min(out.next_change, start - time);
		}
	}
	return out;
}

std::string Encode(json::Object &object) {
	std::ostringstream stream;
	agi::JsonWriter::Write(object, stream);
	std::string response = stream.str();
	response.erase(std::remove(response.begin(), response.end(), '\n'), response.end());
	return response;
}
}

namespace gpui {
RawReply SubtitleLayer(agi::Context *context, json::Object const &request) {
	auto provider = context->project->VideoProvider();
	if (!provider) throw std::runtime_error("no video is open");

	int64_t since = -1;
	if (auto item = request.find("since"); item != request.end())
		since = static_cast<json::Integer const &>(item->second);

	int64_t const time = context->videoController->TimeAtFrame(context->videoController->GetFrameN());
	Visible const visible = VisibleLines(context, time);

	bool const refresh = cache.provider != provider
		|| cache.fingerprint != visible.fingerprint
		|| (visible.animated && cache.time != time);
	if (refresh) {
		cache.provider = provider;
		cache.fingerprint = visible.fingerprint;
		cache.time = time;
		cache.animated = visible.animated;
		++cache.serial;
		cache.rgba.clear();
		cache.x = cache.y = cache.width = cache.height = 0;
		cache.video_width = cache.video_height = 0;
		if (visible.visible) {
			VideoFrame image = provider->GetSubtitles(time / 1000.);
			cache.video_width = image.width;
			cache.video_height = image.height;
			if (image.width > 0 && image.height > 0 && int64_t(image.pitch) >= int64_t(image.width) * 4) {
				// Only what actually drew: a sign costs its own pixels, not the frame.
				int left = image.width;
				int top = image.height;
				int right = -1;
				int bottom = -1;
				for (int y = 0; y < image.height; ++y) {
					unsigned char const *row = image.data.data() + size_t(y) * image.pitch;
					for (int x = 0; x < image.width; ++x) {
						if (row[x * 4 + 3] == 0) continue;
						left = std::min(left, x);
						right = std::max(right, x);
						top = std::min(top, y);
						bottom = std::max(bottom, y);
					}
				}
				if (right >= left && bottom >= top) {
					cache.x = left;
					cache.y = top;
					cache.width = right - left + 1;
					cache.height = bottom - top + 1;
					cache.rgba.resize(size_t(cache.width) * size_t(cache.height) * 4);
					for (int y = 0; y < cache.height; ++y) {
						unsigned char const *src = image.data.data() + size_t(top + y) * image.pitch + size_t(left) * 4;
						unsigned char *dst = cache.rgba.data() + size_t(y) * size_t(cache.width) * 4;
						for (int x = 0; x < cache.width; ++x) {
							unsigned char const b = src[x * 4 + 0];
							unsigned char const g = src[x * 4 + 1];
							unsigned char const r = src[x * 4 + 2];
							unsigned char const a = src[x * 4 + 3];
							// Premultiplied RGBA: CGImage and UpdateLayeredWindow both
							// composite that directly, without another pass per frame.
							dst[x * 4 + 0] = static_cast<unsigned char>(r * a / 255);
							dst[x * 4 + 1] = static_cast<unsigned char>(g * a / 255);
							dst[x * 4 + 2] = static_cast<unsigned char>(b * a / 255);
							dst[x * 4 + 3] = a;
						}
					}
				}
			}
		}
	}

	bool const empty = cache.rgba.empty();
	bool const unchanged = since >= 0 && since == static_cast<int64_t>(cache.serial);
	json::Object head;
	head["ok"] = true;
	head["serial"] = static_cast<int64_t>(cache.serial);
	head["empty"] = empty;
	head["unchanged"] = unchanged;
	head["x"] = static_cast<int64_t>(cache.x);
	head["y"] = static_cast<int64_t>(cache.y);
	head["video_width"] = static_cast<int64_t>(cache.video_width);
	head["video_height"] = static_cast<int64_t>(cache.video_height);
	head["width"] = static_cast<int64_t>(cache.width);
	head["height"] = static_cast<int64_t>(cache.height);
	head["animated"] = visible.animated;
	head["next_change"] = visible.next_change;
	head["time"] = time;
	head["byte_length"] = unchanged ? int64_t(0) : static_cast<int64_t>(cache.rgba.size());
	return {Encode(head), unchanged ? std::vector<unsigned char>() : cache.rgba};
}
}
