// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

#include "gpui_audio.h"

#include "audio_controller.h"
#include "audio_karaoke.h"
#include "ass_dialogue.h"
#include "audio_renderer.h"
#include "audio_renderer_spectrum.h"
#include "audio_rendering_style.h"
#include "audio_renderer_waveform.h"
#include "audio_timing.h"
#include "include/aegisub/context.h"
#include "options.h"
#include "project.h"
#include "selection_controller.h"
#include "video_controller.h"

#include <libaegisub/audio/provider.h>
#include <libaegisub/ass/karaoke.h>
#include <libaegisub/character_count.h>
#include <libaegisub/cajun/writer.h>

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <limits>
#include <sstream>
#include <stdexcept>

namespace {
std::string encode(json::Object const& object) {
	std::ostringstream stream;
	agi::JsonWriter::Write(object, stream);
	auto response = stream.str();
	response.erase(std::remove(response.begin(), response.end(), '\n'), response.end());
	return response;
}

int integer(json::Object const& object, const char *key, int lower, int upper) {
	int64_t number = static_cast<json::Integer const&>(object.at(key));
	if (number < lower || number > upper) throw std::invalid_argument("audio value is out of range");
	return static_cast<int>(number);
}

bool flag(json::Object const& object, const char *key) {
	auto value = object.find(key);
	return value != object.end() && static_cast<json::Boolean const&>(value->second);
}

class StyleRanges final : public AudioRenderingStyleRanges {
public:
	json::Array ranges;

	void AddRange(int start, int end, AudioRenderingStyle style) override {
		if (style != AudioStyle_Inactive && style != AudioStyle_Selected) return;
		json::Object range;
		range["start"] = start;
		range["end"] = end;
		range["selected"] = style == AudioStyle_Selected;
		ranges.emplace_back(std::move(range));
	}
};
}

namespace gpui {
struct Audio::Impl {
	agi::Context *context;
	std::unique_ptr<AudioRendererCanvasProvider> bitmap_provider;
	AudioRenderer renderer;
	std::string mode;
	int gain = 800;
	uint64_t epoch = 1;
	uint64_t render_revision = 1;
	TimeRange marker_range{0, 12000};
	std::vector<agi::signal::Connection> option_connections;
	agi::signal::Connection provider_connection;
	agi::signal::Connection timing_connection;
	agi::signal::Connection selection_connection;
	std::vector<AudioMarker *> dragging;
	uint64_t gesture = 0;

	explicit Impl(agi::Context *context) : context(context) {
		renderer.SetAmplitudeScale(1);
		renderer.SetCacheMaxSize(32 * 1024 * 1024);
		provider_connection = context->project->AddAudioProviderListener([this](agi::AudioProvider *provider) {
			renderer.SetAudioProvider(provider);
			dragging.clear(); ++gesture; ++epoch;
			marker_range = TimeRange(0, 12000);
		});
		timing_connection = context->audioController->AddTimingControllerListener([this] {
			dragging.clear(); ++gesture;
		});
		selection_connection = context->selectionController->AddSelectionListener([this] {
			dragging.clear(); ++gesture;
		});
		renderer.SetAudioProvider(context->project->AudioProvider());
		for (auto name : {"Colour/Audio Display/Spectrum", "Colour/Audio Display/Waveform", "Audio/Display/Waveform Style", "Audio/Renderer/Spectrum/Quality", "Audio/Renderer/Spectrum/FreqCurve", "Audio/Renderer/Spectrum/Memory Max"})
			option_connections.emplace_back(OPT_SUB(name, [this](auto const&) { mode.clear(); ++render_revision; }));
		option_connections.emplace_back(OPT_SUB("Audio/Link", [this](auto const&) { SetGain(gain); }));
		SetGain(gain);
	}

	void SetGain(int value) {
		gain = value;
		if (OPT_GET("Audio/Link")->GetBool()) context->audioController->SetVolume(gain / 100.);
	}

	json::Object State() {
		json::Object result;
		result["loaded"] = !!context->project->AudioProvider();
		result["epoch"] = static_cast<int64_t>(epoch);
		result["render_revision"] = static_cast<int64_t>(render_revision);
		result["snap_distance"] = OPT_GET("Audio/Snap/Distance")->GetInt();
		result["drag_sensitivity"] = OPT_GET("Audio/Start Drag Sensitivity")->GetInt();
		result["lock_scroll"] = OPT_GET("Audio/Lock Scroll on Cursor")->GetBool();
		result["auto_focus"] = OPT_GET("Audio/Auto/Focus")->GetBool();
		result["cursor_time"] = OPT_GET("Audio/Display/Draw/Cursor Time")->GetBool();
		result["boundary_width"] = OPT_GET("Audio/Line Boundaries Thickness")->GetInt();
		auto provider = context->project->AudioProvider();
		result["duration"] = provider ? static_cast<int64_t>(std::min<int64_t>(provider->GetNumSamples() * 1000 / provider->GetSampleRate(), std::numeric_limits<int>::max() - 1)) : 0;
		result["decoded"] = provider ? static_cast<int64_t>(provider->GetDecodedSamples() * 1000 / provider->GetSampleRate()) : 0;
		result["playing"] = context->audioController->IsPlaying();
		result["position"] = context->audioController->GetPlaybackPosition();
		result["rate"] = context->audioController->GetPlaybackRate();
		result["volume"] = static_cast<int>(std::lround(context->audioController->GetVolume() * 100));
		result["gain"] = gain;
		result["linked"] = OPT_GET("Audio/Link")->GetBool();
		result["autocommit"] = OPT_GET("Audio/Auto/Commit")->GetBool();
		result["autonext"] = OPT_GET("Audio/Next Line on Commit")->GetBool();
		result["autoscroll"] = OPT_GET("Audio/Auto/Scroll")->GetBool();
		result["snap"] = OPT_GET("Audio/Snap/Enable")->GetBool();
		result["spectrum"] = OPT_GET("Audio/Spectrum")->GetBool();
		// How many neighbouring lines the waveform draws; the frontend shows this
		// as a direct switch next to the title instead of a preferences trip.
		result["inactive_mode"] = OPT_GET("Audio/Inactive Lines Display Mode")->GetInt();
		result["medusa"] = OPT_GET("Audio/Medusa Timing Hotkeys")->GetBool();
		result["karaoke"] = context->karaoke->IsEnabled();
		json::Array syllables;
		if (context->karaoke->IsEnabled()) {
			auto active = context->selectionController->GetActiveLine();
			result["pending_splits"] = active && context->karaoke->GetModel().GetText() != active->Text.get();
			for (auto const& syllable : context->karaoke->GetModel()) {
				json::Object syl;
				syl["text"] = syllable.text;
				syl["start"] = syllable.start_time;
				syl["duration"] = syllable.duration;
				syl["tag"] = syllable.tag_type;
				syl["characters"] = static_cast<int64_t>(agi::CharacterCount(syllable.text, agi::IGNORE_NONE));
				syllables.emplace_back(std::move(syl));
			}
		}
		result["syllables"] = std::move(syllables);
		json::Array markers;
		StyleRanges ranges;
		if (auto timing = context->audioController->GetTimingController()) {
			timing->GetRenderingStyles(ranges);
			AudioMarkerVector positions;
			timing->GetMarkers(marker_range, positions);
			// Every provider draws with its own pen, and the seconds ticks are
			// dotted while the structural lines are solid; without the pen the
			// frontend cannot tell a second tick from a keyframe or the play
			// cursor, so all of them looked like the same white line.
			for (auto marker : positions) {
				AudioMarkerStyle const style = marker->GetStyle();
				auto const& colour = style.colour;
				char hex[10];
				// agi::Color stores ASS transparency; the wire uses RGBA opacity.
				snprintf(hex, sizeof hex, "#%02x%02x%02x%02x", colour.r, colour.g, colour.b, 255 - colour.a);
				json::Object item;
				item["time"] = marker->GetPosition();
				item["colour"] = std::string(hex);
				item["width"] = marker->GetWidth() > 0 ? marker->GetWidth() : std::max(1, style.width);
				item["dotted"] = style.dotted;
				markers.emplace_back(std::move(item));
			}
		}
		result["markers"] = std::move(markers);
		result["ranges"] = std::move(ranges.ranges);
		TimeRange range = context->audioController->GetPrimaryPlaybackRange();
		result["start"] = range.begin();
		result["end"] = range.end();
		return result;
	}

	void SetMode(std::string const& next_mode) {
		if (mode == next_mode) return;
		std::unique_ptr<AudioRendererCanvasProvider> next;
		if (next_mode == "spectrum") {
			auto spectrum = std::make_unique<AudioSpectrumRenderer>(OPT_GET("Colour/Audio Display/Spectrum")->GetString());
			int const quality = std::clamp<int>(OPT_GET("Audio/Renderer/Spectrum/Quality")->GetInt(), 0, 5);
			int const widths[] = {8, 9, 9, 9, 10, 11};
			int const distances[] = {8, 8, 7, 6, 6, 5};
			spectrum->SetResolution(widths[quality], distances[quality]);
			float const curves[] = {0.001f, 0.125f, 0.333f, 0.425f, 0.999f};
			spectrum->set_reference_frequency_position(curves[std::clamp<int>(OPT_GET("Audio/Renderer/Spectrum/FreqCurve")->GetInt(), 0, 4)]);
			next = std::move(spectrum);
		}
		else if (next_mode == "waveform")
			next = std::make_unique<AudioWaveformRenderer>(OPT_GET("Colour/Audio Display/Waveform")->GetString());
		else throw std::invalid_argument("audio mode must be waveform or spectrum");
		renderer.SetCacheMaxSize(static_cast<size_t>(std::clamp<int64_t>(OPT_GET("Audio/Renderer/Spectrum/Memory Max")->GetInt(), 2, 1024)) * 1024 * 1024);
		renderer.SetRenderer(next.get());
		bitmap_provider = std::move(next);
		mode = next_mode;
	}
};

Audio::Audio(agi::Context *context) : impl(std::make_unique<Impl>(context)) { }
Audio::~Audio() = default;

std::string Audio::Snapshot() { return encode(impl->State()); }

RawReply Audio::Render(json::Object const& request) {
	auto result = impl->State();
	result["ok"] = true;
	auto provider = impl->context->project->AudioProvider();
	if (!provider) {
		result["width"] = static_cast<int64_t>(0);
		result["height"] = static_cast<int64_t>(0);
		result["pixel_format"] = std::string("bgra8");
		result["byte_length"] = static_cast<int64_t>(0);
		result["view_start"] = 0.0;
		result["span"] = static_cast<int64_t>(0);
		return {encode(result), {}};
	}
	int const width = integer(request, "width", 16, 2048);
	int const height = integer(request, "height", 64, 512);
	int const start = integer(request, "start", 0, std::numeric_limits<int>::max() - 1);
	int const span = integer(request, "span", 100, 600000);
	int const gain = integer(request, "gain", 10, 1000);
	impl->SetMode(static_cast<std::string const&>(request.at("mode")));
	impl->renderer.SetHeight(height);
	impl->renderer.SetMillisecondsPerPixel(static_cast<double>(span) / width);
	impl->renderer.SetAmplitudeScale(gain / 100.f);
	int const first_pixel = static_cast<int>(static_cast<int64_t>(start) * width / span);
	AudioCanvas canvas;
	canvas.Reset(width, height);
	impl->renderer.Render(canvas, first_pixel, AudioStyle_Normal);
	std::vector<unsigned char> pixels(canvas.Data(), canvas.Data() + canvas.Size());
	result["width"] = static_cast<int64_t>(width);
	result["height"] = static_cast<int64_t>(height);
	result["pixel_format"] = std::string("bgra8");
	result["byte_length"] = static_cast<int64_t>(pixels.size());
	result["view_start"] = first_pixel * static_cast<double>(span) / width;
	result["span"] = span;
	return {encode(result), std::move(pixels)};
}

std::string Audio::Interact(json::Object const& request) {
	auto context = impl->context;
	if (!context->project->AudioProvider()) throw std::invalid_argument("no audio is loaded");
	auto timing = context->audioController->GetTimingController();
	if (!timing) throw std::invalid_argument("no audio timing controller is available");
	std::string const& action = request.at("action");
	if (action == "view") {
		int const start = integer(request, "start", 0, std::numeric_limits<int>::max() - 600000);
		int const span = integer(request, "span", 250, 600000);
		impl->marker_range = TimeRange(start, start + span);
	}
	else if (action == "down") {
		int const ms = integer(request, "ms", 0, std::numeric_limits<int>::max() - 1);
		int const sensitivity = integer(request, "sensitivity", 0, 10000);
		int const snap = integer(request, "snap", 0, 10000);
		bool const right = flag(request, "right");
		impl->dragging = right ? timing->OnRightClick(ms, flag(request, "control"), sensitivity, snap)
			: timing->OnLeftClick(ms, flag(request, "control"), flag(request, "alt"), sensitivity, snap);
		++impl->gesture;
	}
	else if (action == "drag" || action == "up") {
		uint64_t const gesture = static_cast<json::Integer const&>(request.at("gesture"));
		if (gesture != impl->gesture) throw std::invalid_argument("audio selection changed during this gesture");
		int const ms = integer(request, "ms", 0, std::numeric_limits<int>::max() - 1);
		int const snap = integer(request, "snap", 0, 10000);
		// A click that did not land on a marker returns an empty marker list.
		// The karaoke timing controller indexes m[0] unconditionally, so passing
		// the empty list through aborts the whole application.
		if (!impl->dragging.empty() && (action == "drag" || flag(request, "dragged")))
			timing->OnMarkerDrag(impl->dragging, ms, snap);
		if (action == "up") impl->dragging.clear();
	}
	else if (action == "karaoke_split" || action == "karaoke_merge" || action == "karaoke_accept" || action == "karaoke_cancel" || action == "karaoke_tag") {
		if (!context->karaoke->IsEnabled()) throw std::invalid_argument("karaoke mode is not enabled");
		impl->dragging.clear(); ++impl->gesture;
		if (action == "karaoke_accept") context->karaoke->CommitSplit();
		else if (action == "karaoke_cancel") context->karaoke->RevertSplit();
		else if (action == "karaoke_tag") {
			std::string const& tag = request.at("tag");
			if (tag != "\\k" && tag != "\\kf" && tag != "\\ko") throw std::invalid_argument("invalid karaoke tag");
			context->karaoke->ChangeTagType(tag);
		}
		else {
			auto const& model = context->karaoke->GetModel();
			int const index = integer(request, "syllable", action == "karaoke_merge" ? 1 : 0, static_cast<int>(model.size()) - 1);
			size_t position = 0;
			if (action == "karaoke_split") {
				auto syllable = model.begin() + index;
				int const character = integer(request, "character", 0, agi::CharacterCount(syllable->text, agi::IGNORE_NONE));
				position = agi::IndexOfCharacter(syllable->text, character);
			}
			context->karaoke->EditSplit(index, position, action == "karaoke_merge");
		}
	}
	else if (action == "snap") OPT_SET("Audio/Snap/Enable")->SetBool(flag(request, "enabled"));
	else if (action == "play") context->audioController->PlayPrimaryRange();
	else if (action == "stop") context->audioController->Stop();
	else if (action == "commit") timing->Commit();
	else if (action == "revert") { impl->dragging.clear(); timing->Revert(); }
	else if (action == "gain") impl->SetGain(integer(request, "value", 10, 1000));
	else if (action == "volume") {
		if (OPT_GET("Audio/Link")->GetBool()) throw std::invalid_argument("volume is linked to waveform gain");
		context->audioController->SetVolume(integer(request, "value", 0, 1000) / 100.);
	}
	else if (action == "rate") {
		double const rate = integer(request, "value", 50, 200) / 100.;
		context->videoController->SetPlaybackRate(rate);
	}
	else throw std::invalid_argument("unknown audio action");
	auto result = impl->State();
	result["ok"] = true;
	result["gesture"] = static_cast<int64_t>(impl->gesture);
	return encode(result);
}
}
