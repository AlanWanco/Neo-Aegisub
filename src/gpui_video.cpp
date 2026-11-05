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
#include "video_provider_dummy.h"
#include "visual_tool_vector_clip.h"

#include <libaegisub/cajun/writer.h>
#include <libaegisub/color.h>
#include <libaegisub/util.h>

#include <algorithm>
#include <charconv>
#include <cmath>
#include <optional>
#include <sstream>
#include <stdexcept>
#include <string_view>

#include <wx/frame.h>
#include <wx/sizer.h>

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
	else if (action == "window_zoom") {
		if (!provider) throw std::invalid_argument("open a video before changing zoom");
		double zoom = 0;
		std::string const& text = request.at("value");
		if (!agi::util::try_parse(text, &zoom) || !std::isfinite(zoom) || zoom < .125 || zoom > 10.)
			throw std::invalid_argument("video zoom must be between 0.125 and 10");
		context->videoDisplay->SetWindowZoom(zoom);
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
	result["window_zoom"] = context->videoDisplay->GetWindowZoom();
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
/// Maps the front end's tool names to the existing visual tool commands.
std::string_view ToolCommand(std::string_view tool) {
    if (tool == "cross") return "video/tool/cross";
    if (tool == "drag") return "video/tool/drag";
    if (tool == "rotate/z") return "video/tool/rotate/z";
    if (tool == "rotate/xy") return "video/tool/rotate/xy";
    if (tool == "scale") return "video/tool/scale";
    if (tool == "clip") return "video/tool/clip";
    if (tool == "vector_clip") return "video/tool/vector_clip";
    throw std::invalid_argument("unknown visual tool");
}

/// JSON numbers may arrive as doubles or integers.
double NumberOf(json::UnknownElement const& value) {
    try {
        return static_cast<json::Double const&>(value);
    }
    catch (json::Exception const&) {
        return static_cast<double>(static_cast<json::Integer const&>(value));
    }
}

std::string Encode(json::Object& object) {
    std::ostringstream stream;
    agi::JsonWriter::Write(object, stream);
    auto response = stream.str();
    response.erase(std::remove(response.begin(), response.end(), '\n'), response.end());
    return response;
}
}

struct VideoSurface::Impl {
    agi::Context *context;
    /// Top-level window holding only the canvas; null while closed.
    wxFrame *frame = nullptr;
    /// The video display drawing into frame; null while closed.
    VideoDisplay *display = nullptr;
    /// The display that owned context->videoDisplay before the window opened.
    VideoDisplay *previous = nullptr;
    /// Tool and subtool last requested or observed; applied whenever the window opens.
    std::string tool = "cross";
    int subtool = 0;
    /// Zoom last chosen; unset keeps the legacy display's zoom.
    std::optional<double> zoom;
    agi::signal::Connection provider_connection;

    explicit Impl(agi::Context *context) : context(context) {
        // Unloading the video closes the window: there is nothing left to draw.
        provider_connection = context->project->AddVideoProviderListener([this](AsyncVideoProvider *provider) {
            if (!provider) Close(true);
        });
    }

    void Show() {
        if (frame) {
            frame->Raise();
            return;
        }
        if (!context->project->VideoProvider())
            throw std::invalid_argument("open a video before showing the video window");

        previous = context->videoDisplay;
        frame = new wxFrame(nullptr, wxID_ANY, "Aegisub Video");
        frame->Bind(wxEVT_CLOSE_WINDOW, [this](wxCloseEvent&) { Close(false); });

        // No sub-toolbar: the native window has no toolbar widgets. VideoDisplay and the
        // visual tools accept a null toolbar.
        display = new VideoDisplay(nullptr, nullptr, true, nullptr, frame, context);
        // The constructor already routes context->videoDisplay here; stated again because
        // the tool commands act on whatever that pointer names.
        context->videoDisplay = display;
        auto sizer = new wxBoxSizer(wxVERTICAL);
        sizer->Add(display, 1, wxEXPAND);
        frame->SetSizer(sizer);

        display->SetWindowZoom(zoom.value_or(previous ? previous->GetWindowZoom() : 1.));
        zoom = display->GetWindowZoom();
        ApplyTool();
        frame->Show();
        display->FitWindowToVideo();
        display->SetFocus();
        context->videoController->JumpToFrame(context->videoController->GetFrameN());
    }

    /// Hides the window and restores context->videoDisplay if it still points here.
    /// immediate=false defers destruction (required inside the frame's own close event);
    /// immediate=true destroys now, because ~VideoDisplay reads the context and shutdown
    /// may release the context before idle-time deletion runs.
    void Close(bool immediate) {
        if (!frame) return;
        if (display) {
            zoom = display->GetWindowZoom();
            if (context->videoDisplay == display) {
                if (previous) previous->SetWindowZoom(*zoom);
                context->videoDisplay = previous;
            }
        }
        frame->Hide();
        if (immediate) delete frame;
        else frame->Destroy();
        frame = nullptr;
        display = nullptr;
        previous = nullptr;
    }

    /// Selects the tool through its existing command so the catalogue and the tool state agree.
    void ApplyTool() {
        std::string_view const command = ToolCommand(tool);
        if (!cmd::get(command)->IsActive(context)) cmd::call(command, context);
        if (tool == "vector_clip" && display->ToolIsType(typeid(VisualToolVectorClip)) && display->GetSubTool() != subtool)
            display->SetSubTool(subtool);
    }

    void Configure(json::Object const& request) {
        std::string next_tool = tool;
        int next_subtool = subtool;
        std::optional<double> next_zoom = zoom;
        if (auto it = request.find("tool"); it != request.end()) {
            std::string const& name = it->second;
            ToolCommand(name);
            next_tool = name;
        }
        if (auto it = request.find("subtool"); it != request.end()) {
            if (next_tool != "vector_clip") throw std::invalid_argument("subtool applies only to the vector_clip tool");
            int64_t const value = static_cast<json::Integer const&>(it->second);
            if (value < 0 || value >= VCLIP_LAST) throw std::invalid_argument("vector clip subtool is outside the permitted range");
            next_subtool = static_cast<int>(value);
        }
        if (auto it = request.find("zoom"); it != request.end()) {
            double const value = NumberOf(it->second);
            if (!std::isfinite(value) || value < .125 || value > 10.)
                throw std::invalid_argument("video zoom must be between 0.125 and 10");
            next_zoom = value;
        }
        tool = next_tool;
        subtool = next_subtool;
        zoom = next_zoom;
        if (display) {
            if (zoom) display->SetWindowZoom(*zoom);
            ApplyTool();
            SyncFromDisplay();
        }
    }

    /// Runs one video/tool command in the window, opening the window first if needed, then
    /// records the tool that is actually active. Covers video/tool/vclip/* as well.
    void Command(json::Object const& request) {
        std::string const& name = request.at("name");
        if (!std::string_view(name).starts_with("video/tool/"))
            throw std::invalid_argument("only video/tool commands can be sent to the video window");
        auto const commands = cmd::get_registered_commands();
        if (std::find(commands.begin(), commands.end(), std::string_view(name)) == commands.end())
            throw std::invalid_argument("unknown video tool command");
        Show();
        cmd::call(name, context);
        SyncFromDisplay();
    }

    /// Reads the tool and subtool actually active in the window so stored state never disagrees with it.
    void SyncFromDisplay() {
        tool = ActiveToolName();
        subtool = display->ToolIsType(typeid(VisualToolVectorClip)) ? display->GetSubTool() : 0;
    }

    std::string ActiveToolName() const {
        if (display->ToolIsType(typeid(VisualToolDrag))) return "drag";
        if (display->ToolIsType(typeid(VisualToolRotateZ))) return "rotate/z";
        if (display->ToolIsType(typeid(VisualToolRotateXY))) return "rotate/xy";
        if (display->ToolIsType(typeid(VisualToolScale))) return "scale";
        if (display->ToolIsType(typeid(VisualToolClip))) return "clip";
        if (display->ToolIsType(typeid(VisualToolVectorClip))) return "vector_clip";
        return "cross";
    }

    std::string Reply() const {
        json::Object result;
        result["ok"] = true;
        result["visible"] = frame && frame->IsShown();
        result["renderer"] = "opengl";
        result["tool"] = tool;
        result["subtool"] = subtool;
        double shown_zoom = 1.;
        if (display) shown_zoom = display->GetWindowZoom();
        else if (zoom) shown_zoom = *zoom;
        else if (context->videoDisplay) shown_zoom = context->videoDisplay->GetWindowZoom();
        result["zoom"] = shown_zoom;
        result["frame"] = display ? display->PresentedFrame() : -1;
        result["rendered_frames"] = static_cast<int64_t>(display ? display->RenderedFrames() : 0);
        result["render_failed"] = display ? display->RenderFailed() : false;
        result["error"] = display ? display->RenderError() : std::string();
        return Encode(result);
    }
};

VideoSurface::VideoSurface(agi::Context *context) : impl(std::make_unique<Impl>(context)) { }

VideoSurface::~VideoSurface() {
    if (impl) impl->Close(true);
}

std::string VideoSurface::Handle(json::Object const& request) {
    std::string const& action = request.at("action");
    if (action == "show") impl->Show();
    else if (action == "hide") impl->Close(true);
    else if (action == "configure") impl->Configure(request);
    else if (action == "command") impl->Command(request);
    else if (action == "toggle_move") {
        impl->tool = "drag";
        impl->Show();
        impl->ApplyTool();
        impl->display->ToggleMove();
    }
    else if (action != "state") throw std::invalid_argument("unknown video surface action");
    return impl->Reply();
}

bool VideoSurface::IsVisible() const {
    return impl->frame && impl->frame->IsShown();
}

void VideoSurface::Close() {
    impl->Close(true);
}
}
