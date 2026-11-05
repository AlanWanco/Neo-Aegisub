// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

#include "gpui_search.h"
#include "localisation.h"

#include "ass_dialogue.h"
#include "ass_file.h"
#include "include/aegisub/context.h"
#include "options.h"
#include "search_replace_engine.h"
#include "selection_controller.h"
#include "text_selection_controller.h"

#include <libaegisub/cajun/writer.h>
#include <libaegisub/mru.h>

#include <algorithm>
#include <iterator>
#include <sstream>
#include <stdexcept>

namespace gpui {
std::string Search(agi::Context *context, json::Object const& request) {
	std::string const& action = request.at("action");
	json::Object result;
	auto integer = [&](char const* key, int upper) {
		int64_t const value = static_cast<json::Integer const&>(request.at(key));
		if (value < 0 || value > upper) throw std::invalid_argument("invalid search option");
		return static_cast<int>(value);
	};
	auto boolean = [&](char const* key) { return static_cast<json::Boolean const&>(request.at(key)); };
	if (action != "state") {
		if (action != "select" && action != "preview" && action != "find" && action != "replace" && action != "replace_all" && action != "list")
			throw std::invalid_argument("unknown search action");
		SearchReplaceSettings settings{};
		settings.find = static_cast<std::string const&>(request.at("find"));
		settings.replace_with = static_cast<std::string const&>(request.at("replace_with"));
		settings.field = static_cast<SearchReplaceSettings::Field>(integer("field", 3));
		settings.match_case = boolean("match_case");
		settings.use_regex = boolean("use_regex");
		settings.ignore_comments = boolean("ignore_comments");
		settings.skip_tags = boolean("skip_tags");
		settings.limit_to = boolean("only_selection") ? SearchReplaceSettings::Limit::SELECTED : SearchReplaceSettings::Limit::ALL;
		int const mode = integer("mode", 2);
		if (action == "select" || action == "preview") {
			settings.use_regex = mode == 2;
			settings.exact_match = mode == 0;
		}
		if (action == "list") {
			if (settings.find.empty())
				throw std::invalid_argument("enter text or a regular expression to find");
			auto matches = SearchReplaceEngine::GetMatcher(settings);
			json::Array rows;
			int64_t count = 0;
			for (auto& line : context->ass->Events) {
				auto match = matches(&line, 0);
				if (!match) continue;
				++count;
				// The preview only needs the first match per line; the list is
				// capped so a broad query cannot stall the bridge.
				if (rows.size() >= 200) continue;
				std::string const text = line.Text.get();
				json::Object row;
				row["row"] = static_cast<int64_t>(line.Row + 1);
				row["style"] = line.Style.get();
				row["text"] = text;
				row["after"] = text.substr(0, match.start) + match.replacement + text.substr(match.end);
				rows.emplace_back(std::move(row));
			}
			result["count"] = count;
			result["items"] = std::move(rows);
			result["ok"] = true;
			std::ostringstream stream;
			agi::JsonWriter::Write(result, stream);
			auto response = stream.str();
			response.erase(std::remove(response.begin(), response.end(), '\n'), response.end());
			return response;
		}
		else if (settings.find.empty()) throw std::invalid_argument("enter text or a regular expression to find");
		auto matches = SearchReplaceEngine::GetMatcher(settings);
		if (action == "select" || action == "preview") {
			bool const comments = boolean("comments"), dialogue = boolean("dialogue"), invert = boolean("invert");
			if (!comments && !dialogue) throw std::invalid_argument("choose dialogue lines or comments");
			int const operation = integer("selection_action", 3);
			Selection found;
			for (auto& line : context->ass->Events) {
				if ((line.Comment && !comments) || (!line.Comment && !dialogue)) continue;
				if (invert != static_cast<bool>(matches(&line, 0))) found.insert(&line);
			}
			result["count"] = static_cast<int64_t>(found.size());
			json::Array rows;
			for (auto& line : context->ass->Events) if (found.count(&line)) rows.emplace_back(line.Row + 1);
			result["rows"] = std::move(rows);
			if (action == "select") {
				auto const& old = context->selectionController->GetSelectedSet();
				Selection selection;
				if (operation == 0) selection = std::move(found);
				else if (operation == 1) std::set_union(old.begin(), old.end(), found.begin(), found.end(), std::inserter(selection, selection.end()));
				else if (operation == 2) std::set_difference(old.begin(), old.end(), found.begin(), found.end(), std::inserter(selection, selection.end()));
				else std::set_intersection(old.begin(), old.end(), found.begin(), found.end(), std::inserter(selection, selection.end()));
				auto active = context->selectionController->GetActiveLine();
				if (!selection.empty() && !selection.count(active)) {
					for (auto& line : context->ass->Events) if (selection.count(&line)) { active = &line; break; }
				}
				context->selectionController->SetSelectionAndActive(std::move(selection), active);
			}
			OPT_SET("Tool/Select Lines/Text")->SetString(settings.find);
			OPT_SET("Tool/Select Lines/Mode")->SetInt(mode);
			OPT_SET("Tool/Select Lines/Field")->SetInt(static_cast<int>(settings.field));
			OPT_SET("Tool/Select Lines/Action")->SetInt(operation);
			OPT_SET("Tool/Select Lines/Condition")->SetInt(invert);
			OPT_SET("Tool/Select Lines/Match/Case")->SetBool(settings.match_case);
			OPT_SET("Tool/Select Lines/Match/Comment")->SetBool(comments);
			OPT_SET("Tool/Select Lines/Match/Dialogue")->SetBool(dialogue);
		}
		else if (action == "replace_all") {
			context->search->Configure(settings);
			result["count"] = static_cast<int64_t>(context->search->ReplaceAllMatches());
		}
		else {
			std::vector<AssDialogue *> lines;
			for (auto& line : context->ass->Events) lines.push_back(&line);
			auto active = context->selectionController->GetActiveLine();
			if (!active || lines.empty()) throw std::invalid_argument("no active subtitle line");
			size_t position = std::distance(lines.begin(), std::find(lines.begin(), lines.end(), active));
			if (position == lines.size()) throw std::invalid_argument("active subtitle no longer exists");
			size_t start = static_cast<size_t>(std::max(0, context->textSelectionController->GetSelectionStart()));
			size_t end = static_cast<size_t>(std::max(0, context->textSelectionController->GetSelectionEnd()));
			if (auto range = request.find("range"); range != request.end()) {
				auto const& value = static_cast<json::Array const&>(range->second);
				if (value.size() != 2) throw std::invalid_argument("invalid text selection");
				int64_t const first = static_cast<json::Integer const&>(value[0]), last = static_cast<json::Integer const&>(value[1]);
				auto const& text = active->Text.get();
				auto boundary = [&](int64_t offset) { return offset >= 0 && static_cast<size_t>(offset) <= text.size() && (static_cast<size_t>(offset) == text.size() || (static_cast<unsigned char>(text[offset]) & 0xc0) != 0x80); };
				if (first > last || !boundary(first) || !boundary(last)) throw std::invalid_argument("invalid UTF-8 text selection");
				start = first; end = last;
			}
			bool replaced = false;
			bool const eligible = (settings.limit_to != SearchReplaceSettings::Limit::SELECTED || context->selectionController->GetSelectedSet().count(active)) && (!settings.ignore_comments || !active->Comment);
			if (action == "replace" && eligible) {
				auto match = matches(active, settings.field == SearchReplaceSettings::Field::TEXT ? std::min(start, active->Text.get().size()) : 0);
				if (match) {
					if (settings.field != SearchReplaceSettings::Field::TEXT || (match.start == start && match.end == end)) {
						SearchReplaceEngine::ApplyReplacement(active, match, settings);
						context->ass->Commit(_("replace"), settings.field == SearchReplaceSettings::Field::TEXT ? AssFile::COMMIT_DIAG_TEXT : AssFile::COMMIT_DIAG_META);
						result["replaced"] = true;
						replaced = true;
						end = match.end;
					}
					else {
						context->textSelectionController->SetSelection(match.start, match.end);
						result["found"] = true;
						result["replaced"] = false;
						result["row"] = active->Row + 1;
						result["ok"] = true;
						std::ostringstream stream;
						agi::JsonWriter::Write(result, stream);
						auto response = stream.str(); response.erase(std::remove(response.begin(), response.end(), '\n'), response.end()); return response;
					}
				}
			}
			bool found = false;
			for (size_t step = 0; step <= lines.size(); ++step) {
				auto line = lines[(position + step) % lines.size()];
				if (settings.limit_to == SearchReplaceSettings::Limit::SELECTED && !context->selectionController->GetSelectedSet().count(line)) continue;
				if (settings.ignore_comments && line->Comment) continue;
				if (settings.field != SearchReplaceSettings::Field::TEXT && step == 0) continue;
				size_t const offset = step == 0 && settings.field == SearchReplaceSettings::Field::TEXT ? std::min(end, line->Text.get().size()) : 0;
				if (auto match = matches(line, offset)) {
					if (settings.limit_to == SearchReplaceSettings::Limit::SELECTED) context->selectionController->SetActiveLine(line);
					else context->selectionController->SetSelectionAndActive({line}, line);
					if (settings.field == SearchReplaceSettings::Field::TEXT) context->textSelectionController->SetSelection(match.start, match.end);
					result["row"] = line->Row + 1;
					found = true;
					break;
				}
			}
			result["found"] = found;
			result["replaced"] = replaced;
			context->search->Configure(settings);
		}
		if (action != "select" && action != "preview") {
			config::mru->Add("Find", settings.find);
			if (action != "find") config::mru->Add("Replace", settings.replace_with);
			OPT_SET("Tool/Search Replace/Field")->SetInt(static_cast<int>(settings.field));
			OPT_SET("Tool/Search Replace/Affect")->SetInt(static_cast<int>(settings.limit_to));
			OPT_SET("Tool/Search Replace/Match Case")->SetBool(settings.match_case);
			OPT_SET("Tool/Search Replace/RegExp")->SetBool(settings.use_regex);
			OPT_SET("Tool/Search Replace/Skip Comments")->SetBool(settings.ignore_comments);
			OPT_SET("Tool/Search Replace/Skip Tags")->SetBool(settings.skip_tags);
		}
	}
	json::Object search_defaults;
	auto recent = config::mru->Get("Find");
	search_defaults["find"] = recent->empty() ? std::string() : recent->front().string();
	auto replace = config::mru->Get("Replace");
	search_defaults["replace_with"] = replace->empty() ? std::string() : replace->front().string();
	search_defaults["field"] = OPT_GET("Tool/Search Replace/Field")->GetInt();
	search_defaults["only_selection"] = OPT_GET("Tool/Search Replace/Affect")->GetInt() != 0;
	search_defaults["match_case"] = OPT_GET("Tool/Search Replace/Match Case")->GetBool();
	search_defaults["use_regex"] = OPT_GET("Tool/Search Replace/RegExp")->GetBool();
	search_defaults["ignore_comments"] = OPT_GET("Tool/Search Replace/Skip Comments")->GetBool();
	search_defaults["skip_tags"] = OPT_GET("Tool/Search Replace/Skip Tags")->GetBool();
	result["search_defaults"] = std::move(search_defaults);
	json::Object selection_defaults;
	selection_defaults["find"] = OPT_GET("Tool/Select Lines/Text")->GetString();
	selection_defaults["field"] = OPT_GET("Tool/Select Lines/Field")->GetInt();
	selection_defaults["mode"] = OPT_GET("Tool/Select Lines/Mode")->GetInt();
	selection_defaults["selection_action"] = OPT_GET("Tool/Select Lines/Action")->GetInt();
	selection_defaults["invert"] = OPT_GET("Tool/Select Lines/Condition")->GetInt() != 0;
	selection_defaults["match_case"] = OPT_GET("Tool/Select Lines/Match/Case")->GetBool();
	selection_defaults["comments"] = OPT_GET("Tool/Select Lines/Match/Comment")->GetBool();
	selection_defaults["dialogue"] = OPT_GET("Tool/Select Lines/Match/Dialogue")->GetBool();
	result["selection_defaults"] = std::move(selection_defaults);
	result["ok"] = true;
	json::Array history;
	for (auto const& text : *config::mru->Get("Find")) history.emplace_back(text.string());
	result["history"] = std::move(history);
	std::ostringstream stream;
	agi::JsonWriter::Write(result, stream);
	auto response = stream.str();
	response.erase(std::remove(response.begin(), response.end(), '\n'), response.end());
	return response;
}
}
