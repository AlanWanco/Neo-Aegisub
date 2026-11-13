// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

#include "gpui_subtitles.h"

#include "ass_attachment.h"
#include "ass_dialogue.h"
#include "ass_file.h"
#include "ass_info.h"
#include "ass_style.h"
#include "async_video_provider.h"
#include "include/aegisub/context.h"
#include "include/aegisub/subtitles_provider.h"
#include "project.h"
#include "video_controller.h"
#include "video_frame.h"

#include <libaegisub/background_runner.h>
#include <libaegisub/cajun/writer.h>

#include <algorithm>
#include <cstdint>
#include <functional>
#include <memory>
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

/// The layer's own renderer. It cannot borrow the video provider's one: that
/// instance is drawn from its worker thread, and libass is not thread safe -
/// sharing it crashed the heap inside harfbuzz.
struct Layer {
	std::unique_ptr<SubtitlesProvider> provider;
	uint64_t script = 0;
};
/// Main-thread only, like the cache.
Layer layer;

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

/// Runs a provider's font loading inline: the layer owns its renderer, so
/// there is no worker to hand this to. The sink is never observed.
class InlineRunner final : public agi::BackgroundRunner {
public:
	void Run(std::function<void(agi::ProgressSink *)> task) override {
		struct Sink final : agi::ProgressSink {
			void SetIndeterminate() override { }
			void SetTitle(std::string const &) override { }
			void SetMessage(std::string const &) override { }
			void SetProgress(int64_t, int64_t) override { }
			void Log(std::string const &) override { }
			bool IsCancelled() override { return false; }
		} sink;
		task(&sink);
	}
};

/// Providers keep the runner pointer, so it has to outlive them.
InlineRunner runner;

/// An empty BGRA frame for the renderer to draw on, top row first.
VideoFrame Blank(int width, int height, bool white) {
	VideoFrame frame;
	frame.width = width;
	frame.height = height;
	frame.pitch = width * 4;
	frame.flipped = false;
	frame.data.assign(size_t(frame.pitch) * size_t(height), white ? 255 : 0);
	return frame;
}

/// Every entry the renderer reads, so a style or info edit refreshes the
/// layer even when the visible lines themselves are unchanged.
uint64_t ScriptFingerprint(AssFile *file) {
	uint64_t hash = 1469598103934665603ull;
	for (auto const &entry : file->Info) hash = Mix(hash, entry.GetEntryData());
	for (auto const &entry : file->Styles) hash = Mix(hash, entry.GetEntryData());
	for (auto const &entry : file->Attachments) hash = Mix(hash, entry.GetEntryData());
	for (auto const &entry : file->Events) hash = Mix(hash, entry.GetEntryData());
	return hash;
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

	uint64_t const script = ScriptFingerprint(context->ass.get());
	bool const refresh = cache.provider != provider
		|| layer.script != script
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
			// A preference change swaps the renderer under us: the video
			// provider reloads with the new one, and the layer has to follow.
			// An empty name means the video provider has none yet, which is no
			// reason to rebuild on every request.
			std::string const &loaded = provider->GetSubtitlesProviderName();
			if (!layer.provider || (!loaded.empty() && layer.provider->GetName() != loaded)) {
				layer.provider = SubtitlesProviderFactory::GetProvider(&runner);
				layer.script = 0;
			}
			if (layer.script != script) {
				layer.provider->LoadSubtitles(context->ass.get());
				layer.script = script;
			}
			// Neither libass nor CSRI reports alpha, so unlike mpv the layer
			// has to solve the glyph coverage itself: draw the same frame on
			// black and on white and read the difference back out.
			VideoFrame image = Blank(provider->GetWidth(), provider->GetHeight(), false);
			VideoFrame lit = Blank(provider->GetWidth(), provider->GetHeight(), true);
			double const seconds = double(time) / 1000.;
			layer.provider->DrawSubtitles(image, seconds);
			layer.provider->DrawSubtitles(lit, seconds);
			for (size_t i = 0; i < image.data.size(); i += 4) {
				int const alpha = std::clamp(255 - (int(lit.data[i]) - int(image.data[i])), 0, 255);
				if (alpha == 0) {
					image.data[i] = image.data[i + 1] = image.data[i + 2] = image.data[i + 3] = 0;
					continue;
				}
				for (size_t channel = 0; channel < 3; ++channel)
					image.data[i + channel] = static_cast<unsigned char>(
						std::min(255, int(image.data[i + channel]) * 255 / alpha));
				image.data[i + 3] = static_cast<unsigned char>(alpha);
			}
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
