// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

#include "gpui_visual.h"

#include "ass_file.h"
#include "include/aegisub/context.h"
#include "project.h"
#include "video_controller.h"
#include "visual_render.h"
#include "visual_tool_host.h"
#include "visual_tool_clip.h"
#include "visual_tool_cross.h"
#include "visual_tool_drag.h"
#include "visual_tool_rotatexy.h"
#include "visual_tool_rotatez.h"
#include "visual_tool_scale.h"
#include "visual_tool_vector_clip.h"

#include <libaegisub/cajun/writer.h>

#include <algorithm>
#include <map>
#include <sstream>
#include <stdexcept>
#include <wx/event.h>

namespace {
const std::map<std::string_view, int> vector_modes = {
	{"drag", VCLIP_DRAG}, {"line", VCLIP_LINE}, {"bicubic", VCLIP_BICUBIC},
	{"convert", VCLIP_CONVERT}, {"insert", VCLIP_INSERT}, {"remove", VCLIP_REMOVE},
	{"freehand", VCLIP_FREEHAND}, {"freehand_smooth", VCLIP_FREEHAND_SMOOTH}
};
int number(json::Object const& object, const char *key, int lower, int upper) {
	int64_t value = static_cast<json::Integer const&>(object.at(key));
	if (value < lower || value > upper) throw std::invalid_argument("visual coordinate is outside the permitted range");
	return static_cast<int>(value);
}
bool flag(json::Object const& object, const char *key) {
	auto value = object.find(key);
	return value != object.end() && static_cast<json::Boolean const&>(value->second);
}
}

namespace gpui {
struct Visual::Impl : VisualToolHost {
	agi::Context *context;
	std::unique_ptr<VisualToolBase> tool;
	std::string mode;
	bool captured = false;
	bool left_down = false;
	int width = 0, height = 0;
	agi::signal::Connection provider_connection;

	explicit Impl(agi::Context *context) : context(context) {
		provider_connection = context->project->AddVideoProviderListener([this](auto) {
			tool.reset(); mode.clear(); captured = left_down = false;
		});
	}
	void Render() override { }
	bool HasCapture() const override { return captured; }
	void CaptureMouse() override { captured = true; }
	void ReleaseMouse() override { captured = false; }
	void SetFocus() override { }
	void SetCrosshairCursor(bool) override { }

	void SetMode(std::string const& next) {
		if (next == mode) return;
		std::unique_ptr<VisualToolBase> replacement;
		if (next == "cross") replacement = std::make_unique<VisualToolCross>(this, context);
		else if (next == "drag") replacement = std::make_unique<VisualToolDrag>(this, context);
		else if (next == "rotate/z") replacement = std::make_unique<VisualToolRotateZ>(this, context);
		else if (next == "rotate/xy") replacement = std::make_unique<VisualToolRotateXY>(this, context);
		else if (next == "scale") replacement = std::make_unique<VisualToolScale>(this, context);
		else if (next == "clip") replacement = std::make_unique<VisualToolClip>(this, context);
		else if (next == "vector_clip") replacement = std::make_unique<VisualToolVectorClip>(this, context);
		else throw std::invalid_argument("unknown visual tool");
		if (tool) tool->CancelInteraction();
		tool = std::move(replacement);
		mode = next;
		width = height = 0;
		captured = left_down = false;
	}
};

Visual::Visual(agi::Context *context) : impl(std::make_unique<Impl>(context)) { }
Visual::~Visual() = default;

bool Visual::IsActive(std::string_view command) const {
	if (!command.starts_with("video/tool/")) return false;
	auto name = command.substr(11);
	if (name.starts_with("vclip/")) {
		auto mode = vector_modes.find(name.substr(6));
		return mode != vector_modes.end() && impl->mode == "vector_clip" && impl->tool->GetSubTool() == mode->second;
	}
	return impl->mode == name;
}

void Visual::SelectCommand(std::string_view command) {
	auto name = command.substr(11);
	if (name.starts_with("vclip/")) {
		auto mode = vector_modes.find(name.substr(6));
		if (mode == vector_modes.end()) throw std::invalid_argument("unknown vector clip tool");
		impl->SetMode("vector_clip");
		impl->tool->SetSubTool(mode->second);
	}
	else impl->SetMode(std::string(name));
}

std::string Visual::Interact(json::Object const& request) {
	if (!impl->context->project->VideoProvider()) throw std::invalid_argument("no video is loaded");
	std::string const& action = request.at("action");
	std::string const& mode = request.at("tool");
	impl->SetMode(mode);
	int width = number(request, "width", 16, 8192), height = number(request, "height", 16, 8192);
	if (width != impl->width || height != impl->height) {
		impl->tool->CancelInteraction();
		impl->width = width; impl->height = height;
		impl->tool->SetCanvasSize(width, height);
		impl->tool->SetDisplayArea(0, 0, width, height);
	}
	if (auto subtool = request.find("subtool"); subtool != request.end())
		impl->tool->SetSubTool(number(request, "subtool", 0, VCLIP_LAST - 1));
	if (action == "toggle_move") impl->tool->ToggleMove();
	else if (action == "cancel") { impl->tool->CancelInteraction(); impl->captured = impl->left_down = false; }
	else if (action != "draw") {
		auto const position = wxPoint(number(request, "x", -8192, 16384), number(request, "y", -8192, 16384));
		wxEventType type;
		if (action == "down") { type = wxEVT_LEFT_DOWN; impl->left_down = true; }
		else if (action == "double") { type = wxEVT_LEFT_DCLICK; impl->left_down = true; }
		else if (action == "up") { type = wxEVT_LEFT_UP; impl->left_down = false; }
		else if (action == "move") type = wxEVT_MOTION;
		else if (action == "leave") type = wxEVT_LEAVE_WINDOW;
		else throw std::invalid_argument("unknown visual interaction");
		wxMouseEvent event(type);
		event.SetPosition(position);
		// The last mouse move can be coalesced by the frontend. Finish a drag
		// at the release position before passing the button-up event through.
		if (action == "up" && impl->captured) {
			wxMouseEvent final_move(wxEVT_MOTION);
			final_move.SetPosition(position);
			final_move.SetLeftDown(true);
			final_move.SetShiftDown(flag(request, "shift"));
			final_move.SetAltDown(flag(request, "alt"));
#ifdef __APPLE__
			final_move.SetMetaDown(flag(request, "control"));
#else
			final_move.SetControlDown(flag(request, "control"));
#endif
			impl->tool->OnMouseEvent(final_move);
		}
		event.SetLeftDown(impl->left_down);
		event.SetShiftDown(flag(request, "shift"));
		event.SetAltDown(flag(request, "alt"));
#ifdef __APPLE__
		event.SetMetaDown(flag(request, "control"));
#else
		event.SetControlDown(flag(request, "control"));
#endif
		impl->tool->OnMouseEvent(event);
	}
	VisualRenderCapture capture;
	impl->tool->Draw();
	json::Object response;
	response["ok"] = true;
	response["tool"] = impl->mode;
	response["subtool"] = impl->tool->GetSubTool();
	response["width"] = width;
	response["height"] = height;
	response["captured"] = impl->captured;
	response["commands"] = capture.TakeCommands();
	std::ostringstream stream;
	agi::JsonWriter::Write(response, stream);
	auto text = stream.str();
	text.erase(std::remove(text.begin(), text.end(), '\n'), text.end());
	return text;
}
}
