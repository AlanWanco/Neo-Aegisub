// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

#include "gpui_video.h"

#include "async_video_provider.h"
#include "command/command.h"
#include "include/aegisub/context.h"
#include "options.h"
#include "project.h"
#include "video_controller.h"
#include "video_display.h"
#include "video_frame.h"
#include "video_provider_dummy.h"

#include <libaegisub/cajun/writer.h>
#include <libaegisub/color.h>
#include <libaegisub/util.h>

#include <algorithm>
#include <charconv>
#include <cmath>
#include <sstream>
#include <stdexcept>
#include <string_view>

#include <cstring>
#include <memory>
#include <vector>

namespace gpui {
std::string VideoSettings(agi::Context *context, json::Object const& request) {
	std::string const& action = request.at("action");
	auto integer = [&](char const* name, int lower, int upper) {
		int64_t const value = static_cast<json::Integer const&>(request.at(name));
		if (value < lower || value > upper) throw std::invalid_argument(std::string(name) + " is outside the permitted range");
		return static_cast<int>(value);
	};
	auto provider = context->project->VideoProvider();
	if (action == "dummy") {
		int const width = integer("width", 1, 10000), height = integer("height", 1, 10000), length = integer("length", 2, 36000000);
		std::string const& fps = request.at("fps");
		if (!DummyVideoProvider::TryParseFramerate(fps)) throw std::invalid_argument("enter a valid FPS value or fraction");
		std::string const& color = request.at("color");
		unsigned rgb = 0;
		auto [end, error] = color.size() == 7 && color[0] == '#' ? std::from_chars(color.data() + 1, color.data() + color.size(), rgb, 16) : std::from_chars_result{color.data(), std::errc::invalid_argument};
		if (error != std::errc{} || end != color.data() + color.size()) throw std::invalid_argument("use #RRGGBB for the dummy video color");
		agi::Color value((rgb >> 16) & 255, (rgb >> 8) & 255, rgb & 255);
		bool const pattern = static_cast<json::Boolean const&>(request.at("pattern"));
		context->videoController->Stop();
		context->project->LoadVideo(DummyVideoProvider::MakeFilename(fps, length, width, height, value, pattern));
		OPT_SET("Video/Dummy/FPS String")->SetString(fps);
		OPT_SET("Video/Dummy/Last/Width")->SetInt(width);
		OPT_SET("Video/Dummy/Last/Height")->SetInt(height);
		OPT_SET("Video/Dummy/Last/Length")->SetInt(length);
		OPT_SET("Colour/Video Dummy/Last Colour")->SetColor(value);
		OPT_SET("Video/Dummy/Pattern")->SetBool(pattern);
	}
	else if (action == "jump_frame" || action == "jump_time") {
		if (!provider) throw std::invalid_argument("open a video before seeking");
		int frame = action == "jump_frame" ? integer("frame", 0, provider->GetFrameCount() - 1) : context->videoController->FrameAtTime(integer("time", 0, 35999990));
		context->videoController->Stop();
		context->videoController->JumpToFrame(std::clamp(frame, 0, provider->GetFrameCount() - 1));
	}
	else if (action == "aspect") {
		if (!provider) throw std::invalid_argument("open a video before changing its display ratio");
		std::string const& text = request.at("value");
		double ratio = 0;
		if (!agi::util::try_parse(text, &ratio)) {
			auto split = text.find_first_of(":/xX");
			double numerator = 0, denominator = 0;
			if (split == std::string::npos || !agi::util::try_parse(text.substr(0, split), &numerator) || !agi::util::try_parse(text.substr(split + 1), &denominator) || denominator == 0)
				throw std::invalid_argument("use a decimal, fraction or resolution for the aspect ratio");
			ratio = numerator / denominator;
		}
		if (!std::isfinite(ratio) || ratio < .5 || ratio > 5.) throw std::invalid_argument("aspect ratio must be between 0.5 and 5.0");
		context->videoController->Stop();
		context->videoController->SetAspectRatio(ratio);
	}
	else if (action != "state") throw std::invalid_argument("unknown video settings action");

	provider = context->project->VideoProvider();
	json::Object result;
	result["ok"] = true;
	result["loaded"] = !!provider;
	result["dummy_fps"] = OPT_GET("Video/Dummy/FPS String")->GetString();
	result["dummy_width"] = OPT_GET("Video/Dummy/Last/Width")->GetInt();
	result["dummy_height"] = OPT_GET("Video/Dummy/Last/Height")->GetInt();
	result["dummy_length"] = OPT_GET("Video/Dummy/Last/Length")->GetInt();
	result["dummy_color"] = OPT_GET("Colour/Video Dummy/Last Colour")->GetColor().GetHexFormatted();
	result["dummy_pattern"] = OPT_GET("Video/Dummy/Pattern")->GetBool();
	if (provider) {
		result["path"] = context->project->VideoName().string();
		result["width"] = provider->GetWidth();
		result["height"] = provider->GetHeight();
		result["frames"] = provider->GetFrameCount();
		result["fps"] = provider->GetFPS().FPS();
		result["duration"] = provider->GetFPS().TimeAtFrame(provider->GetFrameCount() - 1);
		result["decoder"] = provider->GetDecoderName();
		result["actual_matrix"] = agi::ycbcr::matrix_to_string(provider->GetRealColorSpace().matrix);
		result["forced_matrix"] = agi::ycbcr::matrix_to_string(provider->GetColorSpace().matrix);
		result["actual_range"] = agi::ycbcr::range_to_string(provider->GetRealColorSpace().range);
		result["forced_range"] = agi::ycbcr::range_to_string(provider->GetColorSpace().range);
		result["aspect"] = context->videoController->GetAspectRatioValue();
		result["frame"] = context->videoController->GetFrameN();
		result["time"] = context->videoController->TimeAtFrame(context->videoController->GetFrameN());
	}
	std::ostringstream stream;
	agi::JsonWriter::Write(result, stream);
	auto response = stream.str();
	response.erase(std::remove(response.begin(), response.end(), '\n'), response.end());
	return response;
}

namespace {
std::string Encode(json::Object& object) {
    std::ostringstream stream;
    agi::JsonWriter::Write(object, stream);
    auto response = stream.str();
    response.erase(std::remove(response.begin(), response.end(), '\n'), response.end());
    return response;
}
}

struct VideoFrames::Impl {
    agi::Context *context;
    /// Newest frame from the provider, composited with subtitles.
    std::shared_ptr<VideoFrame> latest;
    int latest_number = -1;
    /// Bumped whenever `latest` is replaced or dropped.
    int64_t serial = 0;
    bool listening = false;
    agi::signal::Connection provider_connection;

    explicit Impl(agi::Context *context) : context(context) {
        context->videoController->Bind(EVT_FRAME_READY, &Impl::OnFrame, this);
        listening = true;
        // A frame of the previous video must not be shown for the new one.
        provider_connection = context->project->AddVideoProviderListener([this](AsyncVideoProvider *) {
            latest.reset();
            latest_number = -1;
            ++serial;
        });
    }

    void OnFrame(FrameReadyEvent& event) {
        context->videoController->NoteFrameArrived(event.frame_number);
        latest = event.frame;
        latest_number = event.frame_number;
        ++serial;
    }

    void Close() {
        if (listening) context->videoController->Unbind(EVT_FRAME_READY, &Impl::OnFrame, this);
        listening = false;
        provider_connection.Disconnect();
        latest.reset();
        latest_number = -1;
    }
};

VideoFrames::VideoFrames(agi::Context *context) : impl(std::make_unique<Impl>(context)) { }

VideoFrames::~VideoFrames() {
    if (impl) impl->Close();
}

void VideoFrames::Close() {
    impl->Close();
}

RawReply VideoFrames::Frame(json::Object const& request) {
    int64_t since = -1;
    if (auto it = request.find("since"); it != request.end())
        since = static_cast<json::Integer const&>(it->second);

    json::Object result;
    result["ok"] = true;
    result["pixel_format"] = std::string("bgra8");
    result["serial"] = impl->serial;
    result["frame"] = static_cast<int64_t>(impl->latest_number);
    auto const& frame = impl->latest;
    if (!frame || since == impl->serial) {
        result["width"] = static_cast<int64_t>(0);
        result["height"] = static_cast<int64_t>(0);
        result["byte_length"] = static_cast<int64_t>(0);
        result["unchanged"] = !!frame;
        return {Encode(result), {}};
    }

    size_t const width = static_cast<size_t>(frame->width), height = static_cast<size_t>(frame->height);
    size_t const row = width * 4;
    if (frame->pitch < 0 || static_cast<size_t>(frame->pitch) < row || frame->data.size() < static_cast<size_t>(frame->pitch) * height)
        throw std::runtime_error("video frame has an unexpected pixel layout");
    std::vector<unsigned char> pixels(row * height);
    for (size_t y = 0; y < height; ++y) {
        size_t const source = frame->flipped ? height - 1 - y : y;
        std::memcpy(pixels.data() + y * row, frame->data.data() + source * static_cast<size_t>(frame->pitch), row);
    }
    result["width"] = static_cast<int64_t>(width);
    result["height"] = static_cast<int64_t>(height);
    result["byte_length"] = static_cast<int64_t>(pixels.size());
    result["unchanged"] = false;
    return {Encode(result), std::move(pixels)};
}
}
