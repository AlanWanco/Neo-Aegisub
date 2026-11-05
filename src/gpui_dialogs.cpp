// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

#include "gpui_dialogs.h"

#include "aegisublocale.h"
#include "ass_dialogue.h"
#include "ass_exporter.h"
#include "ass_file.h"
#include "ass_style.h"
#include "font_file_lister.h"
#include "command/edit.h"
#include "include/aegisub/context.h"
#include "compat.h"

#include <wx/translation.h>
#include "dialogs.h"
#include "options.h"
#include "version.h"

#include <libaegisub/cajun/writer.h>
#include <libaegisub/charset_conv.h>
#include <libaegisub/log.h>
#include <libaegisub/split.h>

#include <wx/fontenum.h>

#include <filesystem>

#include <algorithm>
#include <sstream>
#include <stdexcept>

namespace {
template <typename T>
json::Array& push(json::Array& array, T value) {
	array.push_back(json::UnknownElement(std::move(value)));
	return array;
}

json::Object about() {
	json::Object result;
	result["version"] = GetAegisubShortVersionString();
	result["long_version"] = GetAegisubLongVersionString();
#ifdef BUILD_CREDIT
	result["build_time"] = GetAegisubBuildTime();
	result["credit"] = GetAegisubBuildCredit();
#endif
	result["official"] = GetIsOfficialRelease();
	return result;
}

json::Array log_entries() {
	json::Array entries;
	static char const* const severity[] = {"exception", "assert", "warning", "info", "debug"};
	auto const messages = agi::log::log->GetMessages();
	// Newest last, but bounded: the log keeps the whole session.
	size_t const begin = messages.size() > 400 ? messages.size() - 400 : 0;
	for (size_t i = begin; i < messages.size(); ++i) {
		auto const& message = messages[i];
		json::Object entry;
		entry["section"] = message.section ? message.section : "";
		entry["message"] = message.message;
		entry["severity"] = json::String(severity[static_cast<int>(message.severity)]);
		push(entries, std::move(entry));
	}
	return entries;
}

json::Array fonts() {
	auto const faces = wxFontEnumerator::GetFacenames();
	json::Array result;
	for (auto const& face : faces) push(result, json::String(from_wx(face)));
	return result;
}

json::Object font_state(agi::Context *context) {
	json::Object result;
	auto current = cmd::CurrentFont(context);
	result["ok"] = true;
	result["available"] = static_cast<bool>(current);
	if (current) {
		result["face"] = current->face;
		result["size"] = static_cast<int64_t>(current->size);
		result["bold"] = current->bold;
		result["italic"] = current->italic;
		result["underline"] = current->underline;
	}
	return result;
}

json::Object export_formats(agi::Context *context) {
	AssExporter exporter(context);
	json::Array formats;
	std::string const& selected = context->ass->Properties.export_filters;
	for (auto const& name : exporter.GetAllFilterNames()) {
		json::Object format;
		format["name"] = name;
		format["description"] = exporter.GetDescription(name);
		bool checked = false;
		if (selected.empty()) checked = true;
		else {
			std::vector<std::string> parts;
			agi::Split(parts, selected, '|');
			checked = std::find(parts.begin(), parts.end(), name) != parts.end();
		}
		format["checked"] = checked;
		push(formats, std::move(format));
	}
	json::Object result;
	result["formats"] = std::move(formats);
	json::Array encodings;
	for (auto const& name : agi::charset::GetEncodingsList<std::vector<std::string>>())
		push(encodings, json::String(name));
	result["encodings"] = std::move(encodings);
	std::string encoding = context->ass->Properties.export_encoding;
	if (encoding.empty()) encoding = "Unicode (UTF-8)";
	result["encoding"] = encoding;
	result["ok"] = true;
	return result;
}

std::vector<AssDialogue *> lines_with_style(agi::Context *context, std::string const& style) {
	std::vector<AssDialogue *> out;
	for (auto &line : context->ass->Events)
		if (line.Style == style) out.push_back(&line);
	return out;
}

json::Object kanji_preview(agi::Context *context, json::Object const& request) {
	json::Object result;
	json::Array styles;
	for (auto const &style : context->ass->Styles) push(styles, json::String(style.name));
	result["styles"] = std::move(styles);

	auto const& source_style = static_cast<json::String const&>(request.at("source"));
	auto const& dest_style = static_cast<json::String const&>(request.at("dest"));
	auto source = lines_with_style(context, source_style);
	auto dest = lines_with_style(context, dest_style);

	json::Array pairs;
	size_t count = std::min(source.size(), dest.size());
	for (size_t i = 0; i < count; ++i) {
		json::Object pair;
		pair["source"] = source[i]->Text.get();
		pair["dest"] = dest[i]->Text.get();
		pair["start"] = static_cast<int64_t>(source[i]->Start);
		pair["end"] = static_cast<int64_t>(source[i]->End);
		push(pairs, std::move(pair));
	}
	result["pairs"] = std::move(pairs);
	result["source_count"] = static_cast<int64_t>(source.size());
	result["dest_count"] = static_cast<int64_t>(dest.size());
	result["ok"] = true;
	return result;
}

json::Object kanji_apply(agi::Context *context, json::Object const& request) {
	auto const& source_style = static_cast<json::String const&>(request.at("source"));
	auto const& dest_style = static_cast<json::String const&>(request.at("dest"));
	auto source = lines_with_style(context, source_style);
	auto dest = lines_with_style(context, dest_style);
	int64_t changed = 0;
	size_t count = std::min(source.size(), dest.size());
	for (size_t i = 0; i < count; ++i) {
		if (dest[i]->Start == source[i]->Start && dest[i]->End == source[i]->End) continue;
		dest[i]->Start = source[i]->Start;
		dest[i]->End = source[i]->End;
		++changed;
	}
	context->ass->Commit(_("Kanji timer"), AssFile::COMMIT_DIAG_TIME);
	json::Object result;
	result["changed"] = changed;
	result["ok"] = true;
	return result;
}

json::Object font_collection(agi::Context *context) {
	FontCollectorStatusCallback callback = [](wxString, int) {};
	FontCollector collector(callback);
	auto paths = collector.GetFontPaths(context->ass.get());
	json::Array fonts;
	for (auto const& path : paths) {
		json::Object font;
		font["path"] = path.string();
		font["name"] = path.filename().string();
		font["size"] = static_cast<int64_t>(std::filesystem::file_size(path.string()));
		push(fonts, std::move(font));
	}
	json::Object result;
	result["fonts"] = std::move(fonts);
	result["ok"] = true;
	return result;
}

json::Object languages() {
	json::Object result;
	json::Array items;
	auto languages = ui_locale.GetAvailableLanguages();
#ifdef WITH_GPUI_DEFAULT
	// Startup pins the language to zh_CN because the GPUI workspace only ships
	// Chinese strings, so offering the other catalogs would be dead UI: picking
	// one only reverts on the next start.
	std::string const active = ui_locale.GetActiveLanguage();
	languages.erase(std::remove_if(languages.begin(), languages.end(),
		[&](auto const& language) { return language.first != active; }), languages.end());
	result["note"] = "GPUI 工作台目前只有中文界面，语言切换暂不可用。";
#endif
	for (auto const& [code, name] : languages) {
		json::Object item;
		item["code"] = code;
		item["name"] = name;
		push(items, std::move(item));
	}
	result["languages"] = std::move(items);
	result["active"] = ui_locale.GetActiveLanguage();
	result["ok"] = true;
	return result;
}
}

namespace gpui {
std::string Dialog(agi::Context *context, json::Object const& request) {
	std::string const& action = request.at("action");
	json::Object result;
	if (action == "about") {
		result = about();
		result["ok"] = true;
	}
	else if (action == "log") {
		result["entries"] = log_entries();
		result["ok"] = true;
	}
	else if (action == "languages") result = languages();
	else if (action == "set_language") {
		std::string const& code = request.at("code");
		OPT_SET("App/Language")->SetString(code);
		result["ok"] = true;
		result["restart"] = code != ui_locale.GetActiveLanguage();
	}
	else if (action == "updates") {
		PerformVersionCheck(true);
		result = GetVersionCheckState();
		result["ok"] = true;
	}
	else if (action == "update_state") {
		result = GetVersionCheckState();
		result["ok"] = true;
	}
	else if (action == "fonts") {
		result["fonts"] = fonts();
		result["ok"] = true;
	}
	else if (action == "font_state") result = font_state(context);
	else if (action == "font_apply") {
		cmd::LineFont font;
		font.face = static_cast<json::String const&>(request.at("face"));
		font.size = static_cast<json::Integer const&>(request.at("size"));
		font.bold = static_cast<json::Boolean const&>(request.at("bold"));
		font.italic = static_cast<json::Boolean const&>(request.at("italic"));
		font.underline = static_cast<json::Boolean const&>(request.at("underline"));
		cmd::ApplyFont(context, font);
		result["ok"] = true;
	}
	else if (action == "export_formats") result = export_formats(context);
	else if (action == "export") {
		std::string const& path = request.at("path");
		AssExporter exporter(context);
		std::string filters;
		for (auto const& value : static_cast<json::Array const&>(request.at("filters"))) {
			auto const& name = static_cast<json::String const&>(value);
			exporter.AddFilter(name);
			if (!filters.empty()) filters += "|";
			filters += name;
		}
		std::string const& encoding = request.at("encoding");
		exporter.Export(path, encoding.c_str());
		context->ass->Properties.export_filters = filters;
		context->ass->Properties.export_encoding = encoding;
		result["ok"] = true;
		result["path"] = path;
	}
	else if (action == "font_collect") result = font_collection(context);
	else if (action == "kanji_preview") result = kanji_preview(context, request);
	else if (action == "kanji_apply") result = kanji_apply(context, request);
	else if (action == "font_copy") {
		std::string const& destination = request.at("destination");
		FontCollectorStatusCallback callback = [](wxString, int) {};
		FontCollector collector(callback);
		int64_t copied = 0;
		std::error_code error;
		std::filesystem::create_directories(destination, error);
		for (auto const& path : collector.GetFontPaths(context->ass.get())) {
			auto target = std::filesystem::path(destination) / path.filename();
			std::filesystem::copy_file(path.string(), target, std::filesystem::copy_options::overwrite_existing, error);
			if (!error) ++copied;
		}
		result["copied"] = copied;
		result["destination"] = destination;
		result["ok"] = true;
	}
	else throw std::invalid_argument("unknown dialog action");

	std::ostringstream stream;
	agi::JsonWriter::Write(result, stream);
	auto response = stream.str();
	response.erase(std::remove(response.begin(), response.end(), '\n'), response.end());
	return response;
}
}
