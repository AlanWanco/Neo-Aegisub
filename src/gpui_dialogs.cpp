// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

#include "gpui_dialogs.h"

#include "localisation.h"
#include "ass_dialogue.h"
#include "ass_exporter.h"
#include "ass_file.h"
#include "ass_karaoke.h"
#include "ass_style.h"
#include "font_file_lister.h"
#include "command/edit.h"
#include "include/aegisub/context.h"

#include "dialogs.h"
#include "options.h"
#include "version.h"

#include <libaegisub/cajun/writer.h>
#include <libaegisub/charset_conv.h>
#include <libaegisub/exception.h>
#include <libaegisub/log.h>
#include <libaegisub/split.h>
#include <libaegisub/karaoke_matcher.h>
#include <libaegisub/unicode.h>


#include <filesystem>

#include <algorithm>
#include <sstream>
#include <stdexcept>
#include <set>
#include <limits>
#include <span>
#include <utility>

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
	auto const faces = GetFontFamilies();
	json::Array result;
	for (auto const& face : faces) push(result, json::String(face));
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

std::vector<AssDialogue *> kanji_lines(agi::Context *context, std::string const& style) {
    std::vector<AssDialogue *> out;
    if (style.empty()) return out;
    for (auto &line : context->ass->Events)
        if (line.Style == style && !line.Text.get().empty()) out.push_back(&line);
    return out;
}

AssDialogue *event_at(agi::Context *context, int id) {
    for (auto &line : context->ass->Events)
        if (line.Id == id) return &line;
    return nullptr;
}

size_t json_count(json::UnknownElement const& value) {
    auto const number = static_cast<json::Integer const&>(value);
    if (number < 0) throw std::invalid_argument("kanji counts must be non-negative");
    return static_cast<size_t>(number);
}

int json_id(json::UnknownElement const& value) {
    auto const number = static_cast<json::Integer const&>(value);
    if (number < 0 || number > std::numeric_limits<int>::max()) throw std::invalid_argument("kanji line id out of range");
    return static_cast<int>(number);
}

AssDialogue *kanji_line(agi::Context *context, json::UnknownElement const& id) {
    auto *line = event_at(context, json_id(id));
    if (!line || line->Text.get().empty()) throw std::invalid_argument("unknown kanji line id");
    return line;
}

json::Array kanji_line_list(agi::Context *context, std::string const& style) {
    json::Array out;
    for (auto *line : kanji_lines(context, style)) {
        json::Object item;
        item["id"] = static_cast<int64_t>(line->Id);
        item["row"] = static_cast<int64_t>(line->Row);
        item["text"] = line->Text.get();
        item["plain"] = line->GetStrippedText();
        item["start"] = static_cast<int64_t>(line->Start);
        item["end"] = static_cast<int64_t>(line->End);
        push(out, std::move(item));
    }
    return out;
}

void set_matcher_input(agi::KaraokeMatcher& matcher, AssDialogue const *source, AssDialogue const *dest) {
    matcher.SetInputData(ParseKaraokeSyllables(source), dest->GetStrippedText());
}

// Sets the current selection to exact counts: source in syllables, destination in graphemes.
void select_counts(agi::KaraokeMatcher& matcher, size_t source, size_t destination) {
    while (matcher.DecreaseSourceMatch()) {}
    while (matcher.DecreaseDestinationMatch()) {}
    for (size_t i = 0; i < source; ++i)
        if (!matcher.IncreaseSourceMatch()) throw std::invalid_argument("kanji source count out of range");
    for (size_t i = 0; i < destination; ++i)
        if (!matcher.IncreaseDestinationMatch()) throw std::invalid_argument("kanji destination count out of range");
}

void replay_groups(agi::KaraokeMatcher& matcher, json::Array const& groups) {
    for (auto const& value : groups) {
        auto const& group = static_cast<json::Object const&>(value);
        size_t const source = json_count(group.at("source"));
        size_t const destination = json_count(group.at("dest"));
        if (source == 0 && destination == 0) throw std::invalid_argument("kanji group selects nothing");
        select_counts(matcher, source, destination);
        matcher.AcceptMatch();
    }
}

bool fully_consumed(agi::KaraokeMatcher const& matcher) {
    return matcher.CurrentSourceSelection().empty() && matcher.CurrentDestinationSelection().empty() &&
        matcher.UnmatchedSource().empty() && matcher.UnmatchedDestination().empty();
}

size_t grapheme_count(std::string_view text) {
    agi::BreakIterator bi;
    bi.set_text(text);
    size_t count = 0;
    for (; !bi.done(); bi.next()) ++count;
    return count;
}

json::Array syllable_list(std::span<const agi::ass::KaraokeSyllable> syllables) {
    json::Array out;
    for (auto const& syl : syllables) {
        json::Object item;
        item["text"] = syl.text;
        item["duration"] = static_cast<int64_t>(syl.duration);
        push(out, std::move(item));
    }
    return out;
}

json::Array group_list(std::span<const agi::KaraokeMatcher::MatchGroup> groups) {
    json::Array out;
    for (auto const& group : groups) {
        std::string source;
        int64_t duration = 0;
        for (auto const& syl : group.src) {
            source += syl.text;
            duration += syl.duration;
        }
        json::Object item;
        item["source"] = source;
        item["dest"] = std::string(group.dst);
        item["duration"] = duration;
        push(out, std::move(item));
    }
    return out;
}

json::Object kanji_preview(agi::Context *context, json::Object const& request) {
    json::Object result;
    json::Array styles;
    for (auto const &style : context->ass->Styles) push(styles, json::String(style.name));
    result["styles"] = std::move(styles);
    result["revision"] = static_cast<json::Integer const&>(request.at("revision"));
    result["interpolate"] = OPT_GET("Tool/Kanji Timer/Interpolation")->GetBool();
    std::string const& source_style = request.at("source");
    std::string const& dest_style = request.at("dest");
    result["source_lines"] = kanji_line_list(context, source_style);
    result["dest_lines"] = kanji_line_list(context, dest_style);
    result["ok"] = true;
    return result;
}

json::Object kanji_match(agi::Context *context, json::Object const& request) {
    auto *source = kanji_line(context, request.at("source_id"));
    auto *dest = kanji_line(context, request.at("dest_id"));
    if (source->Style == dest->Style) throw std::invalid_argument("kanji source and destination styles must differ");

    agi::KaraokeMatcher matcher;
    set_matcher_input(matcher, source, dest);
    replay_groups(matcher, static_cast<json::Array const&>(request.at("groups")));
    if (auto selection = request.find("selection"); selection != request.end()) {
        auto const& sel = static_cast<json::Object const&>(selection->second);
        select_counts(matcher, json_count(sel.at("source")), json_count(sel.at("dest")));
    }
    if (static_cast<json::Boolean const&>(request.at("auto"))) matcher.AutoMatchJapanese();

    auto const current_source = matcher.CurrentSourceSelection();
    auto const current_dest = matcher.CurrentDestinationSelection();
    json::Object selection;
    selection["source"] = static_cast<int64_t>(current_source.size());
    selection["dest"] = static_cast<int64_t>(grapheme_count(current_dest));

    json::Object result;
    result["ok"] = true;
    result["selected_source"] = syllable_list(current_source);
    result["remaining_source"] = syllable_list(matcher.UnmatchedSource());
    result["selected_dest"] = std::string(current_dest);
    result["remaining_dest"] = std::string(matcher.UnmatchedDestination());
    result["selection"] = std::move(selection);
    result["groups"] = group_list(matcher.MatchedGroups());
    result["output"] = matcher.GetOutputLine();
    result["complete"] = fully_consumed(matcher);
    return result;
}

json::Object kanji_apply(agi::Context *context, json::Object const& request) {
    bool const interpolate = static_cast<json::Boolean const&>(request.at("interpolate"));
    auto const& entries = static_cast<json::Array const&>(request.at("entries"));

    // Validate and generate every entry before mutating anything.
    std::vector<std::pair<AssDialogue *, std::string>> outputs;
    std::set<AssDialogue const *> destinations;
    for (auto const& value : entries) {
        auto const& entry = static_cast<json::Object const&>(value);
        auto *source = kanji_line(context, entry.at("source_id"));
        auto *dest = kanji_line(context, entry.at("dest_id"));
        if (source->Style == dest->Style) throw std::invalid_argument("kanji source and destination styles must differ");
        if (!destinations.insert(dest).second) throw std::invalid_argument("duplicate kanji destination line");

        agi::KaraokeMatcher matcher;
        set_matcher_input(matcher, source, dest);
        replay_groups(matcher, static_cast<json::Array const&>(entry.at("groups")));
        if (!fully_consumed(matcher)) throw std::invalid_argument("kanji entry does not consume every source and destination count");
        outputs.emplace_back(dest, matcher.GetOutputLine());
    }

    int64_t changed = 0;
    for (auto& [dest, output] : outputs) {
        if (dest->Text.get() == output) continue;
        dest->Text = output;
        ++changed;
    }
    if (changed) context->ass->Commit(_("Kanji timer"), AssFile::COMMIT_DIAG_TEXT);
    OPT_SET("Tool/Kanji Timer/Interpolation")->SetBool(interpolate);

    json::Object result;
    result["changed"] = changed;
    result["ok"] = true;
    return result;
}

json::Object font_collection(agi::Context *context) {
	FontCollectorStatusCallback callback = [](std::string, int) {};
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
	std::vector<std::pair<std::string, std::string>> languages;
	languages.emplace_back("en_US", agi::i18n::LanguageName("en_US"));
	for (auto const& code : agi::i18n::AvailableLanguages())
		languages.emplace_back(code, agi::i18n::LanguageName(code));
#ifdef WITH_GPUI_DEFAULT
	// Startup pins the language to zh_CN because the GPUI workspace only ships
	// Chinese strings, so offering the other catalogs would be dead UI: picking
	// one only reverts on the next start.
	std::string const active = agi::i18n::ActiveLanguage();
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
	result["active"] = agi::i18n::ActiveLanguage();
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
		result["restart"] = code != agi::i18n::ActiveLanguage();
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
		try {
			exporter.Export(path, encoding.c_str());
			context->ass->Properties.export_filters = filters;
			context->ass->Properties.export_encoding = encoding;
			result["ok"] = true;
			result["path"] = path;
		}
		catch (agi::UserCancelException const&) {
			result["ok"] = false;
			result["cancelled"] = true;
		}
	}
	else if (action == "font_collect") result = font_collection(context);
    else if (action == "kanji_preview") result = kanji_preview(context, request);
    else if (action == "kanji_match") result = kanji_match(context, request);
    else if (action == "kanji_apply") result = kanji_apply(context, request);
	else if (action == "font_copy") {
		std::string const& destination = request.at("destination");
		FontCollectorStatusCallback callback = [](std::string, int) {};
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
