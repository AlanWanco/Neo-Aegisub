// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

#include "gpui_preferences.h"

#include "ass_style_storage.h"
#include "audio_provider_factory.h"
#include "command/command.h"
#include "compat.h"
#include "include/aegisub/audio_player.h"
#include "include/aegisub/hotkey.h"
#include "include/aegisub/subtitles_provider.h"
#include "libresrc/libresrc.h"
#include "options.h"
#include "ui_theme.h"
#include "video_provider_manager.h"

#ifdef WITH_PORTAUDIO
#include "audio_player_portaudio.h"
#endif

#include <libaegisub/cajun/reader.h>
#include <libaegisub/cajun/writer.h>
#include <libaegisub/io.h>
#include <libaegisub/hotkey.h>
#include <libaegisub/path.h>

#include <algorithm>
#include <charconv>
#include <cmath>
#include <memory>
#include <set>
#include <sstream>
#include <stdexcept>
#include <wx/intl.h>

namespace {
struct Field {
	const char *group;
	const char *label;
	const char *name;
	double lower = 0;
	double upper = 2147483647;
	bool directory = false;
};
Field const fields[] = {
#include "gpui_preferences_fields.inc"
};

std::string encode(json::Object result) {
	std::ostringstream stream;
	agi::JsonWriter::Write(result, stream);
	auto response = stream.str();
	response.erase(std::remove(response.begin(), response.end(), '\n'), response.end());
	return response;
}

void backup(const char *filename) {
	auto path = config::path->Decode(std::string("?user/") + filename);
	if (agi::fs::FileExists(path)) agi::fs::Copy(path, path.string() + ".backup-before-gpui");
}

std::set<std::string, std::less<>> const& valid_hotkeys() {
	static std::set<std::string, std::less<>> const keys = [] {
		std::set<std::string, std::less<>> keys;
		// Use the same names and modifier order as the wx keyboard handler.
		for (int mask = 0; mask < 8; ++mask) {
			int modifiers = (mask & 1 ? wxMOD_CMD : 0) | (mask & 2 ? wxMOD_ALT : 0) | (mask & 4 ? wxMOD_SHIFT : 0);
			for (int code = 8; code <= WXK_SPECIAL20; ++code) {
				if (code == WXK_SHIFT || code == WXK_ALT || code == WXK_CONTROL || (code >= 'a' && code <= 'z')) continue;
				auto candidate = hotkey::keypress_to_str(code, modifiers);
				if (!candidate.empty() && !hotkey::keypress_to_str(code, wxMOD_NONE).empty()) keys.insert(candidate);
			}
		}
		return keys;
	}();
	return keys;
}

bool valid_hotkey(std::string const& key) {
	return valid_hotkeys().count(key) != 0;
}

/// Read a hotkey configuration file in either the current or the pre-3.1 format.
agi::hotkey::Hotkey::HotkeyMap load_hotkey_map(std::string const& path) {
	auto stream = agi::io::Open(agi::fs::path(path));
	json::UnknownElement root;
	try {
		json::Reader::Read(root, *stream);
	}
	catch (json::Exception const& error) {
		throw std::invalid_argument(std::string("不是有效的 JSON 快捷键文件：") + error.what());
	}
	try {
		return agi::hotkey::ParseHotkeyMap(static_cast<json::Object const&>(root));
	}
	catch (std::bad_cast const&) {
		throw std::invalid_argument("不是快捷键配置文件：应为“上下文 → 命令 → 快捷键列表”的对象");
	}
}

std::vector<std::string> choices(std::string const& name) {
	if (name == "App/Theme") return {"跟随系统", "浅色", "深色"};
	if (name == "App/Auto/Load Linked Files") return {"从不", "总是", "询问"};
	if (name == "Audio/Inactive Lines Display Mode") return {"不显示", "上一行", "上一行与下一行", "全部"};
	if (name == "Audio/Display/Waveform Style") return {"最大值", "最大值与平均值"};
	if (name == "Video/Scroll Action" || name == "Video/Ctrl Scroll Action" || name == "Video/Shift Scroll Action") return {"调整视频框", "反向调整视频框", "缩放画面", "反向缩放画面", "平移画面", "交换 X/Y 平移", "不操作"};
	if (name == "Video/Default Zoom") {
		std::vector<std::string> result;
		for (int i = 1; i <= 24; ++i) { std::ostringstream value; value << i * 12.5 << '%'; result.push_back(value.str()); }
		return result;
	}
	if (name == "Video/PlayRes Mismatch") return {"从不", "询问", "总是重采样"};
	if (name == "Video/No LayoutRes in Script") return {"从不", "询问", "总是设置", "总是使用脚本分辨率"};
	if (name == "Video/LayoutRes Mismatch") return {"从不", "比例变化时", "总是"};
	if (name == "Video/No YCbCr Matrix in Script" || name == "Video/YCbCr Matrix Mismatch") return {"从不", "询问", "总是设置"};
	if (name == "Subtitle Format/EBU STL/TV Standard") return {"23.976 (标记为 24)", "24", "25 (PAL)", "29.97 非丢帧 (标记为 30)", "29.97 丢帧", "30 (NTSC 单色)"};
	if (name == "Subtitle Format/EBU STL/Text Encoding") return {"ISO 6937-2", "ISO 8859-5 西里尔", "ISO 8859-6 阿拉伯", "ISO 8859-7 希腊", "ISO 8859-8 希伯来", "UTF-8 (非标准)"};
	if (name == "Subtitle Format/EBU STL/Line Wrapping Mode") return {"自动换行", "均衡换行", "超长时报错", "跳过超长行"};
	if (name == "Subtitle Format/EBU STL/Display Standard") return {"开放字幕", "一级图文电视字幕", "二级图文电视字幕"};
	if (name == "Automation/Trace Level") return {"0: Fatal", "1: Error", "2: Warning", "3: Hint", "4: Debug", "5: Trace"};
	if (name == "Automation/Autoreload Mode") return {"不重载", "项目脚本", "自动加载脚本", "全部脚本"};
	if (name == "Audio/Cache/Type") return {"无缓存", "内存", "硬盘"};
	if (name == "Audio/Renderer/Spectrum/Quality") return {"普通", "较好", "高", "极高"};
	if (name == "Audio/Renderer/Spectrum/FreqCurve") return {"线性", "展开", "中等", "压缩", "对数"};
	if (name == "Provider/Audio/FFmpegSource/Decode Error Handling") return {"忽略", "清除", "停止", "中止"};
	if (name == "Provider/FFmpegSource/Log Level") return {"Quiet", "Panic", "Fatal", "Error", "Warning", "Info", "Verbose", "Debug"};
	if (name == "Path/Screenshot") return {};
	if (name == "Audio/Provider") return GetAudioProviderNames();
	if (name == "Audio/Player") return AudioPlayerFactory::GetClasses();
	if (name == "Video/Provider") return VideoProviderFactory::GetClasses();
	if (name == "Subtitle/Provider") return SubtitlesProviderFactory::GetClasses();
	if (name == "Colour/Audio Display/Spectrum" || name == "Colour/Audio Display/Waveform") return OPT_GET("Audio/Colour Schemes")->GetListString();
	if (name == "Audio/Downmixer") return {"ConvertToMono", "GetLeftChannel", "GetRightChannel"};
	if (name.starts_with("Subtitle Format/") && name.ends_with("/Default Style Catalog")) {
		auto result = AssStyleStorage::GetCatalogs();
		result.push_back("Default"); result.push_back(OPT_GET(name)->GetString());
		std::sort(result.begin(), result.end()); result.erase(std::unique(result.begin(), result.end()), result.end()); return result;
	}
#ifdef WITH_PORTAUDIO
	if (name == "Player/Audio/PortAudio/Device Name") {
		std::vector<std::string> result;
		for (auto const& device : PortAudioPlayer::GetOutputDevices()) result.push_back(from_wx(device));
		return result;
	}
#endif
	return {};
}

std::string value(agi::OptionValue const* option) {
	switch (option->GetType()) {
		case agi::OptionType::Bool: return option->GetBool() ? "1" : "0";
		case agi::OptionType::Int: return std::to_string(option->GetInt());
		case agi::OptionType::Double: { std::ostringstream out; out.precision(15); out << option->GetDouble(); return out.str(); }
		case agi::OptionType::String: return option->GetString();
		case agi::OptionType::Color: return option->GetColor().GetHexFormatted(true);
		default: throw std::invalid_argument("unsupported preference type");
	}
}

agi::Color color(std::string const& value) {
	if ((value.size() != 7 && value.size() != 9) || value[0] != '#') throw std::invalid_argument("use #RRGGBB or #RRGGBBAA for colors");
	auto component = [&](size_t offset) {
		unsigned number = 0;
		auto [end, error] = std::from_chars(value.data() + offset, value.data() + offset + 2, number, 16);
		if (error != std::errc{} || end != value.data() + offset + 2) throw std::invalid_argument("invalid hexadecimal color");
		return static_cast<unsigned char>(number);
	};
	return agi::Color(component(1), component(3), component(5), value.size() == 9 ? component(7) : 0);
}

std::unique_ptr<agi::OptionValue> parse(Field const& field, std::string const& text) {
	auto option = OPT_GET(field.name);
	switch (option->GetType()) {
		case agi::OptionType::Bool:
			if (text != "0" && text != "1") throw std::invalid_argument("use 0 or 1 for boolean options");
			return std::make_unique<agi::OptionValueBool>(field.name, text == "1");
		case agi::OptionType::Int: {
			int64_t number = 0;
			auto [end, error] = std::from_chars(text.data(), text.data() + text.size(), number);
			auto values = choices(field.name);
			if (error != std::errc{} || end != text.data() + text.size() || number < field.lower || number > field.upper || (!values.empty() && number >= static_cast<int64_t>(values.size()))) throw std::invalid_argument("integer preference is outside its allowed range");
			return std::make_unique<agi::OptionValueInt>(field.name, number);
		}
		case agi::OptionType::Double: {
			size_t consumed = 0;
			double number = std::stod(text, &consumed);
			if (consumed != text.size() || !std::isfinite(number) || number < field.lower || number > field.upper) throw std::invalid_argument("decimal preference is outside its allowed range");
			return std::make_unique<agi::OptionValueDouble>(field.name, number);
		}
		case agi::OptionType::Color: return std::make_unique<agi::OptionValueColor>(field.name, color(text));
		case agi::OptionType::String: {
			auto values = choices(field.name);
			if (!values.empty() && std::find(values.begin(), values.end(), text) == values.end()) throw std::invalid_argument("choose an available preference value");
			return std::make_unique<agi::OptionValueString>(field.name, text);
		}
		default: throw std::invalid_argument("unsupported preference type");
	}
}
}

namespace gpui {
std::string Appearance() {
	json::Object colors;
	json::Array default_colors;
	for (auto key : {"Colour/Subtitle Grid/Standard", "Colour/Subtitle Grid/Background/Background", "Colour/Subtitle Grid/Selection", "Colour/Subtitle Grid/Background/Selection", "Colour/Subtitle Grid/Background/Comment", "Colour/Subtitle Grid/Background/Selected Comment", "Colour/Subtitle Grid/Background/Inframe", "Colour/Subtitle Grid/Lines", "Colour/Subtitle Grid/Header", "Colour/Subtitle/Background", "Colour/Subtitle/Syntax/Normal", "Colour/Audio Display/Play Cursor", "Colour/Audio Display/Line boundary Start", "Colour/Audio Display/Line boundary End", "Colour/Audio Display/Line Boundary Inactive Line"}) {
		auto color = ui::OptionColour(key);
		if (std::string_view(key).starts_with("Colour/Subtitle") && OPT_GET(key)->IsDefault()) default_colors.emplace_back(key);
		colors[key] = static_cast<int64_t>((uint32_t(color.Red()) << 24) | (uint32_t(color.Green()) << 16) | (uint32_t(color.Blue()) << 8) | color.Alpha());
	}
	json::Object result;
	result["colors"] = std::move(colors);
	result["default_colors"] = std::move(default_colors);
	result["grid_font"] = OPT_GET("Subtitle/Grid/Font Face")->GetString();
	result["grid_size"] = OPT_GET("Subtitle/Grid/Font Size")->GetInt();
	result["edit_font"] = OPT_GET("Subtitle/Edit Box/Font Face")->GetString();
	result["edit_size"] = OPT_GET("Subtitle/Edit Box/Font Size")->GetInt();
	result["soft_line_break"] = OPT_GET("Subtitle/Edit Box/Soft Line Break")->GetBool();
	result["highlight_visible"] = OPT_GET("Subtitle/Grid/Highlight Subtitles in Frame")->GetBool();
	result["focus_grid"] = OPT_GET("Subtitle/Grid/Focus Allow")->GetBool();
	result["grid_widths"] = OPT_GET("Subtitle/Grid/GPUI Column Widths")->GetString();
	result["seek_on_double_click"] = OPT_GET("Video/Seek On Double Click")->GetBool();
	result["loop_playback"] = OPT_GET("Video/Loop Playback")->GetBool();
	return encode(std::move(result));
}

namespace {
/// Copy one JSON leaf into an option, dispatching on the option's own type so a
/// stale or wrong-typed entry is skipped instead of aborting the whole import.
bool apply_element(agi::OptionValue *option, json::UnknownElement const& element) {
	try {
		switch (option->GetType()) {
			case agi::OptionType::Bool: option->SetBool(static_cast<bool const&>(element)); return true;
			case agi::OptionType::Int: option->SetInt(static_cast<json::Integer const&>(element)); return true;
			case agi::OptionType::Double: option->SetDouble(static_cast<double const&>(element)); return true;
			case agi::OptionType::String: option->SetString(static_cast<std::string const&>(element)); return true;
			case agi::OptionType::Color: option->SetColor(agi::Color(static_cast<std::string const&>(element))); return true;
			case agi::OptionType::ListColor: {
				std::vector<agi::Color> items;
				for (auto const& entry : static_cast<json::Array const&>(element))
					items.emplace_back(static_cast<std::string const&>(static_cast<json::Object const&>(entry).at("color")));
				option->SetListColor(items);
				return true;
			}
			case agi::OptionType::ListString: {
				std::vector<std::string> items;
				for (auto const& entry : static_cast<json::Array const&>(element)) items.emplace_back(static_cast<std::string const&>(entry));
				option->SetListString(items);
				return true;
			}
			case agi::OptionType::ListInt: {
				std::vector<int64_t> items;
				for (auto const& entry : static_cast<json::Array const&>(element)) items.emplace_back(static_cast<json::Integer const&>(entry));
				option->SetListInt(items);
				return true;
			}
			case agi::OptionType::ListDouble: {
				std::vector<double> items;
				for (auto const& entry : static_cast<json::Array const&>(element)) items.emplace_back(static_cast<double const&>(entry));
				option->SetListDouble(items);
				return true;
			}
			case agi::OptionType::ListBool: {
				std::vector<bool> items;
				for (auto const& entry : static_cast<json::Array const&>(element)) items.emplace_back(static_cast<bool const&>(entry));
				option->SetListBool(items);
				return true;
			}
		}
	}
	catch (...) {
		// Type mismatch: the entry came from a build with a different option type.
	}
	return false;
}

/// Walk the imported document, applying every path the running build knows.
void import_object(json::Object const& object, std::string const& prefix, int &applied, int &skipped) {
	for (auto const& [key, item] : object) {
		std::string const path = prefix.empty() ? key : prefix + "/" + key;
		agi::OptionValue *option = nullptr;
		try {
			option = config::opt->Get(path);
		}
		catch (...) {
			option = nullptr;
		}
		if (option) {
			if (apply_element(option, item)) ++applied;
			else ++skipped;
			continue;
		}
		try {
			import_object(static_cast<json::Object const&>(item), path, applied, skipped);
		}
		catch (...) {
			++skipped;
		}
	}
}
}

std::string Preferences(json::Object const& request) {
	std::string const& action = request.at("action");
	int imported = 0, skipped = 0;
	if (action == "grid_widths") {
		std::string const& widths = request.at("widths");
		backup("config.json");
		OPT_SET("Subtitle/Grid/GPUI Column Widths")->SetString(widths);
		config::opt->Flush();
		return "{\"ok\":true}";
	}
	if (action == "apply") {
		std::vector<std::unique_ptr<agi::OptionValue>> changes;
		for (auto const& [name, item] : static_cast<json::Object const&>(request.at("fields"))) {
			auto field = std::find_if(std::begin(fields), std::end(fields), [&](auto const& field) { return name == field.name; });
			if (field == std::end(fields)) throw std::invalid_argument("unknown preference");
			auto patch = parse(*field, static_cast<std::string const&>(item));
			if (value(patch.get()) != value(OPT_GET(name))) changes.push_back(std::move(patch));
		}
		if (!changes.empty()) {
			backup("config.json");
			for (auto const& change : changes) OPT_SET(change->GetName())->Set(change.get());
			config::opt->Flush();
		}
	}
	else if (action == "reset") {
		std::vector<agi::OptionValue *> reset;
		for (auto const& name : static_cast<json::Array const&>(request.at("names"))) {
			std::string const& key = name;
			if (std::none_of(std::begin(fields), std::end(fields), [&](auto const& field) { return key == field.name; })) throw std::invalid_argument("unknown preference");
			reset.push_back(OPT_SET(key));
		}
		backup("config.json");
		for (auto option : reset) option->Reset();
		config::opt->Flush();
	}
	else if (action == "import") {
		std::string const& path = request.at("path");
		auto stream = agi::io::Open(agi::fs::path(path));
		json::UnknownElement root;
		try {
			json::Reader::Read(root, *stream);
		}
		catch (json::Exception const& error) {
			throw std::invalid_argument(std::string("不是有效的 JSON 配置文件：") + error.what());
		}
		backup("config.json");
		import_object(static_cast<json::Object const&>(root), "", imported, skipped);
		config::opt->Flush();
	}
	else if (action != "state") throw std::invalid_argument("unknown preferences action");
	json::Array options;
	for (auto const& field : fields) {
		auto option = OPT_GET(field.name);
		json::Object item;
		item["name"] = field.name;
		item["label"] = from_wx(wxGetTranslation(field.label));
		item["group"] = field.group;
		item["type"] = option->GetType() == agi::OptionType::Bool ? "bool" : option->GetType() == agi::OptionType::Int ? "int" : option->GetType() == agi::OptionType::Double ? "double" : option->GetType() == agi::OptionType::Color ? "color" : "string";
		item["value"] = value(option);
		item["minimum"] = field.lower;
		item["maximum"] = field.upper;
		item["directory"] = field.directory;
		item["is_default"] = option->IsDefault();
		std::string_view key = field.name;
		static const std::set<std::string_view> live_gpui = {
			"App/Theme", "App/Show Toolbar", "Subtitle/Grid/Font Face", "Subtitle/Grid/Font Size",
			"Subtitle/Edit Box/Font Face", "Subtitle/Edit Box/Font Size", "Subtitle/Edit Box/Soft Line Break",
			"Subtitle/Grid/Focus Allow", "Subtitle/Grid/Highlight Subtitles in Frame",
			"Audio/Snap/Distance", "Audio/Start Drag Sensitivity",
			"Video/Scroll Action", "Video/Ctrl Scroll Action", "Video/Shift Scroll Action",
			"Colour/Subtitle Grid/Standard", "Colour/Subtitle Grid/Background/Background",
			"Colour/Subtitle Grid/Selection", "Colour/Subtitle Grid/Background/Selection",
			"Colour/Subtitle Grid/Background/Comment", "Colour/Subtitle Grid/Background/Selected Comment",
			"Colour/Subtitle Grid/Background/Inframe", "Colour/Subtitle Grid/Lines", "Colour/Subtitle Grid/Header",
			"Colour/Subtitle/Background", "Colour/Subtitle/Syntax/Normal",
			"Colour/Audio Display/Play Cursor", "Colour/Audio Display/Line boundary Start",
			"Colour/Audio Display/Line boundary End", "Colour/Audio Display/Line Boundary Inactive Line",
			"Colour/Audio Display/Spectrum", "Colour/Audio Display/Waveform",
			"Audio/Display/Waveform Style", "Audio/Renderer/Spectrum/Quality",
			"Audio/Renderer/Spectrum/FreqCurve", "Audio/Renderer/Spectrum/Memory Max"
		};
		std::string note;
		if (live_gpui.count(key)) note = "GPUI 工作区即时应用，并与 wx 界面共享设置。";
		else if (key == "Video/Default Zoom") note = "新建窗口时使用该默认值；当前视频可用缩放按钮调整。";
		else if (key == "Audio/Provider" || key == "Audio/Player" || key == "Video/Provider" || key == "Subtitle/Provider" || key.starts_with("Provider/") || key.starts_with("Player/") || key.starts_with("Audio/Cache/")) note = "后端或缓存设置在重新打开媒体时生效；对应的 GPUI 控件行为仍待验证。";
		else note = "设置已保存；对应的 GPUI 控件行为仍待移植。";
		item["note"] = std::move(note);
		json::Array options_list;
		for (auto const& value : choices(field.name)) options_list.emplace_back(value);
		item["choices"] = std::move(options_list);
		options.emplace_back(std::move(item));
	}
	json::Object result;
	result["ok"] = true;
	result["imported"] = static_cast<int64_t>(imported);
	result["skipped"] = static_cast<int64_t>(skipped);
	result["options"] = std::move(options);
	return encode(std::move(result));
}

std::string Hotkeys(json::Object const& request) {
	std::string const& action = request.at("action");
	int64_t imported = 0, skipped = 0;
	if (action != "state") {
		auto map = hotkey::inst->GetHotkeyMap();
		if (action == "reset") {
			agi::hotkey::Hotkey defaults("", GET_DEFAULT_CONFIG(default_hotkey));
			map = defaults.GetHotkeyMap();
		}
		else if (action == "import") {
			std::set<std::string_view, std::less<>> known;
			for (auto name : cmd::get_registered_commands()) known.insert(name);
			for (auto const& [command, combo] : load_hotkey_map(request.at("path"))) {
				if (known.count(command) == 0 || combo.Context().empty() || !valid_hotkey(std::string(combo.Str()))) {
					++skipped;
					continue;
				}
				auto same_slot = [&](auto const& item) {
					return item.second.Context() == combo.Context() && item.second.Str() == combo.Str();
				};
				if (std::any_of(map.begin(), map.end(), [&](auto const& item) { return same_slot(item) && item.first == command; })) {
					++skipped;
					continue;
				}
				for (auto it = map.begin(); it != map.end();) {
					if (same_slot(*it)) it = map.erase(it);
					else ++it;
				}
				map.emplace(command, combo);
				++imported;
			}
		}
		else {
			std::string const& context = request.at("context"), key = request.at("key"), command = request.at("command");
			if (context.empty() || !valid_hotkey(key)) throw std::invalid_argument("enter a context and a shortcut using the recorder or the existing key names");
			if (action == "set") {
				if (!command.empty()) cmd::get(command);
				bool replace = false;
				if (auto item = request.find("replace"); item != request.end()) replace = static_cast<json::Boolean const&>(item->second);
				for (auto it = map.begin(); it != map.end();) {
					if (it->second.Context() == context && it->second.Str() == key) {
						if (!replace && it->second.CmdName() != command) throw std::invalid_argument("shortcut is already assigned in this context; confirm replacement");
						it = map.erase(it);
					}
					else ++it;
				}
				map.emplace(command, agi::hotkey::Combo(context, command, key));
			}
			else if (action == "remove") {
				auto it = std::find_if(map.begin(), map.end(), [&](auto const& item) { return item.second.Context() == context && item.second.Str() == key && item.second.CmdName() == command; });
				if (it == map.end()) throw std::invalid_argument("shortcut no longer exists");
				map.erase(it);
			}
			else throw std::invalid_argument("unknown hotkey action");
		}
		backup("hotkey.json");
		hotkey::inst->SetHotkeyMap(std::move(map));
	}
	json::Array bindings;
	for (auto const& [name, combo] : hotkey::inst->GetHotkeyMap()) {
		json::Object item;
		item["command"] = name;
		item["context"] = combo.Context();
		item["key"] = combo.Str();
		bindings.emplace_back(std::move(item));
	}
	json::Object result;
	result["ok"] = true;
	if (action == "import") {
		result["imported"] = imported;
		result["skipped"] = skipped;
	}
	result["bindings"] = std::move(bindings);
	return encode(std::move(result));
}
}
