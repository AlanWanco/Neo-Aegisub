// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

#include "gpui_command_catalog.h"

#include "localisation.h"
#include "gpui_visual.h"

#include "auto4_base.h"
#include "command/command.h"
#include "include/aegisub/context.h"
#include "include/aegisub/hotkey.h"
#include "include/aegisub/menu.h"
#include "options.h"
#include "selection_controller.h"

#include <libaegisub/cajun/reader.h>
#include <libaegisub/cajun/writer.h>
#include <libaegisub/hotkey.h>


#include <algorithm>
#include <sstream>

namespace gpui {
std::string_view CommandSurface(std::string_view name) {
	if (name == "app/about" || name == "app/log" || name == "app/updates") return "native";
	if (name.starts_with("video/zoom/") || name == "video/reset_pan" || name == "video/show_overscan") return "native";
	if (name == "app/toggle/toolbar" || name == "subtitle/spellcheck" || name == "subtitle/open/charset" || name == "subtitle/open/autosave" ||
		name == "audio/save/clip" || name == "keyframe/save" || name == "timecode/save" || name == "app/options" ||
		name == "am/manager" || name == "am/meta" || name == "am/reload" || name == "am/reload/autoload" ||
		name == "tool/style/assistant" || name.starts_with("tool/styling_assistant/") ||
		name == "tool/translation_assistant" || name.starts_with("tool/translation_assistant/") ||
		name.starts_with("video/tool/") || name.starts_with("audio/go_to") || name.starts_with("audio/scroll/") ||
		name.starts_with("grid/tags/") || name == "grid/tag/cycle_hiding" || name == "time/shift" ||
		name == "tool/resampleres" || name == "tool/line/select" || name == "edit/line/paste/over" ||
		name == "edit/find_replace" || name == "subtitle/find" || name == "subtitle/find/next" ||
		name == "tool/time/postprocess" || name == "keyframe/open" || name == "timecode/open" ||
		name == "subtitle/attachment" || name == "subtitle/properties" || name == "subtitle/open" ||
		name == "video/open" || name == "video/open/dummy" || name == "video/jump" ||
		name == "video/aspect/custom" || name == "video/details" || name == "audio/open" ||
		name == "tool/style/manager" || name == "subtitle/save" || name == "subtitle/save/as" ||
		name == "audio/view/waveform" || name == "audio/view/spectrum" ||
		name == "audio/opt/vertical_link" || name == "app/language" ||
		name.starts_with("edit/color/") || name == "edit/font" || name == "subtitle/open/video" ||
		name == "tool/export" || name == "tool/font_collector" || name == "tool/time/kanji" ||
		name == "subtitle/close" || name == "video/copy_coordinates" || name == "video/detach")
		return "native";
	if (name == "app/launch/gpui") return "internal";
	if (name.starts_with("automation/lua/")) return "automation";
	return "shared";
}

std::string CommandCatalog(agi::Context *context, Visual const& visual) {
	std::set<std::string> macros;
	for (auto macro : config::global_scripts->GetMacros()) macros.insert(macro->name());
	for (auto macro : context->local_scripts->GetMacros()) macros.insert(macro->name());

	json::Array commands;
	for (auto name : cmd::get_registered_commands()) {
		// Other open documents have their own local macros in the global registry.
		if (name.starts_with("automation/lua/") && !macros.count(std::string(name))) continue;
		auto command = cmd::get(name);
		int const flags = command->Type();
		json::Object entry;
		entry["name"] = name;
		entry["label"] = command->StrDisplay(context);
		entry["help"] = command->StrHelp();
		entry["surface"] = CommandSurface(name);
		bool enabled = command->Validate(context);
		if (name.starts_with("tool/translation_assistant/") || name.starts_with("tool/styling_assistant/")) enabled = context->selectionController->GetActiveLine() != nullptr;
		// The GPUI workspace owns these toggles now; the wx frame's own state
		// would otherwise disable the mode that is not currently active.
		if (name.starts_with("app/display/")) enabled = true;
		if (name.starts_with("recent/")) {
			auto const last_slash = name.rfind('/');
			if (last_slash == 6) enabled = false; // submenu group, not an operation
			else {
				static const std::map<std::string_view, const char *> types = {
					{"audio", "Audio"}, {"keyframes", "Keyframes"}, {"subtitle", "Subtitle"},
					{"timecodes", "Timecodes"}, {"video", "Video"}
				};
				auto type = types.find(name.substr(7, last_slash - 7));
				int const index = std::stoi(std::string(name.substr(last_slash + 1)));
				enabled = type != types.end() && index >= 0 && static_cast<size_t>(index) < config::mru->Get(type->second)->size();
				if (enabled) entry["label"] = config::mru->GetEntry(type->second, index).filename().string();
			}
		}
		entry["enabled"] = enabled;
		entry["active"] = name.starts_with("video/tool/") ? visual.IsActive(name) :
			(flags & (cmd::COMMAND_RADIO | cmd::COMMAND_TOGGLE)) && command->IsActive(context);
		entry["shortcut"] = hotkey::get_hotkey_str_first("Default", name);
		commands.emplace_back(std::move(entry));
	}

	json::Object result;
	result["ok"] = true;
	result["commands"] = std::move(commands);
	json::Array bindings;
	for (auto const& [name, combo] : hotkey::inst->GetHotkeyMap()) {
		json::Object binding;
		binding["command"] = name;
		binding["context"] = combo.Context();
		binding["key"] = combo.Str();
		bindings.emplace_back(std::move(binding));
	}
	result["bindings"] = std::move(bindings);
	result["always_hotkeys"] = OPT_GET("Audio/Medusa Timing Hotkeys")->GetBool();
	// This is the same merged platform/user configuration used by wx menus,
	// but wx translates the labels while building its menu bar. Without the
	// same step here the GPUI menu shows raw English entries like "Appearance".
	// json::UnknownElement is move-only, so the translated copy is rebuilt
	// field by field (menu entries only ever hold strings).
	json::Object menu_config;
	for (auto const& [name, value] : menu::GetConfig()) {
		json::Array items;
		for (auto const& item : static_cast<json::Array const&>(value)) {
			auto const& object = static_cast<json::Object const&>(item);
			std::string context;
			if (auto ctx = object.find("context"); ctx != object.end())
				context = static_cast<json::String const&>(ctx->second);
			json::Object entry;
			for (auto const& [key, field] : object) {
				auto text = static_cast<json::String const&>(field);
				if (key == "text")
					text = agi::i18n::TranslateContext(context.c_str(), text.c_str());
				entry[key] = json::String(std::move(text));
			}
			items.emplace_back(std::move(entry));
		}
		menu_config[name] = std::move(items);
	}
	std::ostringstream menus;
	agi::JsonWriter::Write(menu_config, menus);
	std::ostringstream out;
	agi::JsonWriter::Write(result, out);
	std::string response = out.str();
	response.pop_back();
	response += ",\"menus\":" + menus.str() + "}";
	// The bridge uses one JSON value per line. JsonWriter pretty-prints, so
	// remove formatting newlines (newlines in strings are already escaped).
	response.erase(std::remove(response.begin(), response.end(), '\n'), response.end());
	return response;
}
}
