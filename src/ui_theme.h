// Copyright (c) 2026, Aegisub contributors
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

#pragma once

#include "options.h"

#include <libaegisub/color.h>

#include <cstdlib>
#include <string>
#include <string_view>

#ifdef _WIN32
#include <windows.h>
// windows.h defines GetMessage as GetMessageW, which hides agi::Exception::GetMessage.
#undef GetMessage
#elif defined(__APPLE__)
#include <CoreFoundation/CoreFoundation.h>
#endif

/// The host does not draw widgets, but the front end asks for the colours the
/// workspace should use, and the theme commands write the shared option.
namespace ui {
enum class ThemeMode { System, Light, Dark };

inline ThemeMode Mode() {
	auto value = OPT_GET("App/Theme")->GetInt();
	if (value == static_cast<int>(ThemeMode::Light)) return ThemeMode::Light;
	if (value == static_cast<int>(ThemeMode::Dark)) return ThemeMode::Dark;
	return ThemeMode::System;
}

inline void SetThemeMode(ThemeMode mode) {
	OPT_SET("App/Theme")->SetInt(static_cast<int>(mode));
}

inline agi::Color ColorFromHex(unsigned value) {
	return agi::Color((value >> 16) & 255, (value >> 8) & 255, value & 255);
}

inline bool SystemIsDark() {
#ifdef _WIN32
	DWORD value = 1;
	DWORD size = sizeof(value);
	if (RegGetValueW(HKEY_CURRENT_USER,
		L"Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize",
		L"AppsUseLightTheme", RRF_RT_REG_DWORD, nullptr, &value, &size) == ERROR_SUCCESS)
		return value == 0;
	return false;
#elif defined(__APPLE__)
	bool dark = false;
	CFPropertyListRef style = CFPreferencesCopyAppValue(CFSTR("AppleInterfaceStyle"), CFSTR("NSGlobalDomain"));
	if (style) {
		if (CFGetTypeID(style) == CFStringGetTypeID())
			dark = CFStringCompare(static_cast<CFStringRef>(style), CFSTR("Dark"), 0) == kCFCompareEqualTo;
		CFRelease(style);
	}
	return dark;
#else
	if (char const *theme = std::getenv("GTK_THEME"))
		return std::string_view(theme).find("dark") != std::string_view::npos;
	if (char const *colors = std::getenv("COLORFGBG")) {
		std::string_view view(colors);
		auto semicolon = view.rfind(';');
		if (semicolon != std::string_view::npos) {
			int background = std::atoi(std::string(view.substr(semicolon + 1)).c_str());
			return background >= 0 && background < 8;
		}
	}
	return false;
#endif
}

inline bool IsDark() {
	auto mode = Mode();
	return mode == ThemeMode::Dark || (mode == ThemeMode::System && SystemIsDark());
}

inline agi::Color Canvas() { return ColorFromHex(IsDark() ? 0x191F20 : 0xE7ECEA); }
inline agi::Color Surface() { return ColorFromHex(IsDark() ? 0x22292A : 0xF5F8F7); }
inline agi::Color Raised() { return ColorFromHex(IsDark() ? 0x2B3434 : 0xFFFFFF); }
inline agi::Color Text() { return ColorFromHex(IsDark() ? 0xE5EFEC : 0x233631); }
inline agi::Color Muted() { return ColorFromHex(IsDark() ? 0xA6B8B2 : 0x526761); }
inline agi::Color Accent() { return ColorFromHex(IsDark() ? 0x78D8B9 : 0x08785D); }
inline agi::Color Selection() { return ColorFromHex(IsDark() ? 0x23483B : 0xD5EEE4); }
inline agi::Color Border() { return ColorFromHex(IsDark() ? 0x3B4846 : 0xCCD7D2); }

// Only replace the historical defaults. Explicit user colour choices remain
// authoritative.
inline agi::Color OptionColour(std::string const& key) {
	struct DefaultColour {
		std::string_view key;
		unsigned legacy, light, dark;
	};
	static constexpr DefaultColour defaults[] = {
		{"Colour/Subtitle Grid/Active Border", 0xFF5BEF, 0x126C58, 0x7BD7BC},
		{"Colour/Subtitle Grid/Background/Background", 0xFFFFFF, 0xFFFFFF, 0x242A2B},
		{"Colour/Subtitle Grid/Background/Comment", 0xD8DEF5, 0xF1F3F7, 0x30343D},
		{"Colour/Subtitle Grid/Background/Inframe", 0xFFFDEA, 0xF9F4DF, 0x39382B},
		{"Colour/Subtitle Grid/Background/Selected Comment", 0xD3EEEE, 0xE1EBED, 0x30454A},
		{"Colour/Subtitle Grid/Background/Selection", 0xCEFFE7, 0xDDEFE8, 0x29473E},
		{"Colour/Subtitle Grid/Collision", 0xFF0000, 0xAE3346, 0xFF9EA8},
		{"Colour/Subtitle Grid/Header", 0xA5CFE7, 0xEDF3F0, 0x303A37},
		{"Colour/Subtitle Grid/Left Column", 0xC4ECC9, 0xF5F8F6, 0x2A3130},
		{"Colour/Subtitle Grid/Lines", 0xBEBEBE, 0xE4EBE7, 0x394340},
		{"Colour/Subtitle Grid/Selection", 0x000000, 0x174B3E, 0xDCF5EA},
		{"Colour/Subtitle Grid/Standard", 0x000000, 0x253C38, 0xE4ECE9},
		{"Colour/Subtitle/Background", 0xFFFFFF, 0xFFFFFF, 0x242A2B},
		{"Colour/Subtitle/Syntax/Normal", 0x000000, 0x253C38, 0xE4ECE9},
		{"Colour/Subtitle/Syntax/Comment", 0x000000, 0x566B65, 0xAABAB5},
		{"Colour/Subtitle/Syntax/Drawing Command", 0x000000, 0x126C58, 0x7BD7BC},
		{"Colour/Subtitle/Syntax/Drawing X", 0x5A2828, 0x9B4551, 0xF2A3AD},
		{"Colour/Subtitle/Syntax/Drawing Y", 0x285A28, 0x326B42, 0xA0D3A8},
		{"Colour/Subtitle/Syntax/Brackets", 0x1432FF, 0x365FA6, 0x9ABCF5},
		{"Colour/Subtitle/Syntax/Error", 0xC80000, 0xAE3346, 0xFF9EA8},
		{"Colour/Subtitle/Syntax/Karaoke Template", 0x8000C0, 0x78539F, 0xCBB0EE},
		{"Colour/Subtitle/Syntax/Karaoke Variable", 0x8000C0, 0x78539F, 0xCBB0EE},
		{"Colour/Subtitle/Syntax/Line Break", 0xA0A0A0, 0x63766F, 0xACBBB4},
		{"Colour/Subtitle/Syntax/Parameters", 0x285A28, 0x326B42, 0xA0D3A8},
		{"Colour/Subtitle/Syntax/Slashes", 0xFF00C8, 0x9D467C, 0xE9ACD3},
		{"Colour/Subtitle/Syntax/Tags", 0x5A5A5A, 0x566B65, 0xAABAB5},
		{"Colour/Subtitle/Syntax/Background/Error", 0xFFC8C8, 0xFBE6E9, 0x59333A},
	};
	auto color = OPT_GET(key)->GetColor();
	unsigned value = (unsigned(color.r) << 16) | (unsigned(color.g) << 8) | color.b;
	for (auto const& entry : defaults) {
		if (entry.key != key || entry.legacy != value || color.a != 0) continue;
		bool const themed = Mode() != ThemeMode::System || IsDark();
		if (key == "Colour/Subtitle Grid/Background/Background" || key == "Colour/Subtitle/Background")
			return themed ? Raised() : ColorFromHex(entry.light);
		if (key == "Colour/Subtitle Grid/Background/Selection")
			return themed ? Selection() : ColorFromHex(entry.light);
		return ColorFromHex(IsDark() ? entry.dark : entry.light);
	}
	if (Mode() == ThemeMode::System) return color;
	return agi::Color(color.r, color.g, color.b, 255 - color.a);
}
}
