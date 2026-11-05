// Copyright (c) 2010, Amar Takhar <verm@aegisub.org>
//
// Permission to use, copy, modify, and distribute this software for any
// purpose with or without fee is hereby granted, provided that the above
// copyright notice and this permission notice appear in all copies.
//
// THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
// WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
// MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
// ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
// WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
// ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
// OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.

#include <libaegisub/hotkey.h>

#include "include/aegisub/hotkey.h"

#include "libresrc/libresrc.h"
#include "command/command.h"
#include "options.h"

#include <libaegisub/path.h>

#include <boost/range/algorithm/find.hpp>
#include <boost/range/iterator_range.hpp>

namespace {
	const char *added_hotkeys_7035[][3] = {
		{"audio/play/line", "Audio", "R"},
		{nullptr}
	};

	const char *added_hotkeys_7070[][3] = {
		{"edit/color/primary", "Subtitle Edit Box", "Alt-1"},
		{"edit/color/secondary", "Subtitle Edit Box", "Alt-2"},
		{"edit/color/outline", "Subtitle Edit Box", "Alt-3"},
		{"edit/color/shadow", "Subtitle Edit Box", "Alt-4"},
		{nullptr}
	};

	const char *added_hotkeys_shift_back[][3] = {
		{"edit/line/duplicate/shift_back", "Default", "Ctrl-Shift-D"},
		{nullptr}
	};

#ifdef __APPLE__
	const char *added_hotkeys_minimize[][3] = {
		{"app/minimize", "Default", "Ctrl-M"},
		{nullptr}
	};
#endif

	void migrate_hotkeys(const char *added[][3]) {
		auto hk_map = hotkey::inst->GetHotkeyMap();
		bool changed = false;

		for (size_t i = 0; added[i][0]; ++i) {
			agi::hotkey::Combo combo(added[i][1], added[i][0], added[i][2]);

			if (hotkey::inst->HasHotkey(combo.Context(), combo.Str()))
				continue;

			hk_map.insert(make_pair(std::string(added[i][0]), std::move(combo)));
			changed = true;
		}

		if (changed)
			hotkey::inst->SetHotkeyMap(std::move(hk_map));
	}
}

namespace hotkey {

agi::hotkey::Hotkey *inst = nullptr;
void init() {
	inst = new agi::hotkey::Hotkey(
		config::path->Decode("?user/hotkey.json"),
		GET_DEFAULT_CONFIG(default_hotkey));

	auto migrations = OPT_GET("App/Hotkey Migrations")->GetListString();

	if (boost::find(migrations, "7035") == end(migrations)) {
		migrate_hotkeys(added_hotkeys_7035);
		migrations.emplace_back("7035");
	}

	if (boost::find(migrations, "7070") == end(migrations)) {
		migrate_hotkeys(added_hotkeys_7070);
		migrations.emplace_back("7070");
	}

	if (boost::find(migrations, "edit/line/duplicate/shift_back") == end(migrations)) {
		migrate_hotkeys(added_hotkeys_shift_back);
		migrations.emplace_back("edit/line/duplicate/shift_back");
	}

	if (boost::find(migrations, "duplicate -> split") == end(migrations)) {
		auto hk_map = hotkey::inst->GetHotkeyMap();
		for (auto const& hotkey : boost::make_iterator_range(hk_map.equal_range("edit/line/duplicate/shift"))) {
			auto combo = agi::hotkey::Combo(hotkey.second.Context(), "edit/line/split/before", hotkey.second.Str());
			hk_map.emplace(combo.CmdName(), combo);
		}
		for (auto const& hotkey : boost::make_iterator_range(hk_map.equal_range("edit/line/duplicate/shift_back"))) {
			auto combo = agi::hotkey::Combo(hotkey.second.Context(), "edit/line/split/after", hotkey.second.Str());
			hk_map.emplace(combo.CmdName(), combo);
		}

		hk_map.erase("edit/line/duplicate/shift");
		hk_map.erase("edit/line/duplicate/shift_back");

		hotkey::inst->SetHotkeyMap(std::move(hk_map));
		migrations.emplace_back("duplicate -> split");
	}

#ifdef __APPLE__
	if (boost::find(migrations, "app/minimize") == end(migrations)) {
		migrate_hotkeys(added_hotkeys_minimize);
		migrations.emplace_back("app/minimize");
	}
#endif

	OPT_SET("App/Hotkey Migrations")->SetListString(std::move(migrations));
}

void clear() {
	delete inst;
}

std::vector<std::string> const& key_names() {
	static std::vector<std::string> const names = {
		"Backspace", "Tab", "Enter", "Escape", "Space", "Delete", "Shift",
		"Alt", "Control", "Pause", "End", "Home", "Left", "Up", "Right",
		"Down", "Print", "Insert", "KP_0", "KP_1", "KP_2", "KP_3", "KP_4",
		"KP_5", "KP_6", "KP_7", "KP_8", "KP_9", "Asterisk", "Plus",
		"Hyphen", "Period", "Slash", "F1", "F2", "F3", "F4", "F5", "F6",
		"F7", "F8", "F9", "F10", "F11", "F12", "F13", "F14", "F15", "F16",
		"F17", "F18", "F19", "F20", "F21", "F22", "F23", "F24", "Num_Lock",
		"Scroll_Lock", "PageUp", "PageDown", "KP_Space", "KP_Tab",
		"KP_Enter", "KP_F1", "KP_F2", "KP_F3", "KP_F4", "KP_Home",
		"KP_Left", "KP_Up", "KP_Right", "KP_Down", "KP_PageUp",
		"KP_PageDown", "KP_End", "KP_Begin", "KP_insert", "KP_Delete",
		"KP_Equal", "KP_Multiply", "KP_Add", "KP_Subtract", "KP_Decimal",
		"KP_Divide"
	};
	return names;
}

std::vector<std::string> get_hotkey_strs(std::string_view context, std::string_view command) {
	return inst->GetHotkeys(context, command);
}

std::string_view get_hotkey_str_first(std::string_view context, std::string_view command) {
	return inst->GetHotkey(context, command);
}

} // namespace hotkey
