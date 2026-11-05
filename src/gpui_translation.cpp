// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

#include "gpui_translation.h"
#include "localisation.h"

#include "ass_dialogue.h"
#include "ass_file.h"
#include "include/aegisub/context.h"
#include "options.h"
#include "selection_controller.h"
#include "video_controller.h"

#include <libaegisub/cajun/writer.h>
#include <libaegisub/unicode.h>

#include <algorithm>
#include <boost/algorithm/string/replace.hpp>
#include <sstream>
#include <stdexcept>

namespace gpui {
std::string Translation(agi::Context *context, json::Object const& request) {
	std::string const& action = request.at("action");
	bool skip_whitespace = OPT_GET("Tool/Translation Assistant/Skip Whitespace")->GetBool();
	if (auto skip = request.find("skip_whitespace"); skip != request.end()) skip_whitespace = static_cast<json::Boolean const&>(skip->second);
	bool seek = false;
	if (auto value = request.find("seek"); value != request.end()) seek = static_cast<json::Boolean const&>(value->second);

	struct Block {
		AssDialogue *line;
		size_t index;
		std::string text;
	};
	std::vector<Block> available;
	auto collect = [&] {
		available.clear();
		for (auto& line : context->ass->Events) {
			auto blocks = line.ParseTags();
			for (size_t i = 0; i < blocks.size(); ++i) {
				if (blocks[i]->GetType() != AssBlockType::PLAIN) continue;
				auto text = blocks[i]->GetText();
				if (skip_whitespace && agi::unicode::is_whitespace(text)) continue;
				available.push_back({&line, i, std::move(text)});
			}
		}
	};
	collect();	AssDialogue *line = context->selectionController->GetActiveLine();
	int64_t index = 0;
	if (auto value = request.find("block"); value != request.end()) index = static_cast<json::Integer const&>(value->second);
	if (index < 0) throw std::invalid_argument("invalid translation block");
	if (auto value = request.find("id"); value != request.end()) {
		int64_t id = static_cast<json::Integer const&>(value->second);
		auto match = std::find_if(context->ass->Events.begin(), context->ass->Events.end(), [&](auto const& candidate) { return candidate.Id == id; });
		if (match == context->ass->Events.end()) throw std::invalid_argument("translation line no longer exists");
		line = &*match;
	}
	auto current = std::find_if(available.begin(), available.end(), [&](auto const& block) { return block.line == line && block.index == static_cast<size_t>(index); });
	bool boundary = false;
	bool exhausted = false;
	if (action == "start") {
		context->videoController->Stop();
		current = std::find_if(available.begin(), available.end(), [&](auto const& block) { return !line || block.line->Row >= line->Row; });
		if (current == available.end()) { current = available.begin(); exhausted = available.empty(); }
	}
	else if (action == "next" || action == "prev") {
		if (current == available.end()) throw std::invalid_argument("translation block changed; refresh the assistant");
		if (action == "next") {
			if (std::next(current) == available.end()) boundary = exhausted = true;
			else ++current;
		}
		else {
			if (current == available.begin()) boundary = true;
			else --current;
		}
	}
	else if (action == "preview" || action == "commit") {
		if (current == available.end()) throw std::invalid_argument("translation block changed; refresh the assistant");
		std::string const& expected = request.at("expected");
		if (current->text != expected) throw std::invalid_argument("source text changed; refresh before translating it");
		std::string translated = request.at("text");
		boost::replace_all(translated, "\r\n", "\\N");
		boost::replace_all(translated, "\r", "\\N");
		boost::replace_all(translated, "\n", "\\N");
		auto blocks = current->line->ParseTags();
		blocks[current->index] = std::make_unique<AssDialogueBlockPlain>(translated);
		current->line->UpdateText(blocks);
		context->ass->Commit(_("translation assistant"), AssFile::COMMIT_DIAG_TEXT);
		AssDialogue translated_block;
		translated_block.Text = translated;
		size_t const inserted_blocks = translated.empty() ? 0 : translated_block.ParseTags().size();
		auto edited_line = current->line;
		size_t const next_index = current->index + (action == "commit" ? inserted_blocks : 0);
		collect();
		current = std::find_if(available.begin(), available.end(), [&](auto const& candidate) {
			return candidate.line->Row > edited_line->Row || (candidate.line == edited_line && candidate.index >= next_index);
		});
		if (current == available.end()) {
			boundary = exhausted = true;
			if (!available.empty()) current = std::prev(available.end());
		}
	}
	else if (action != "state") throw std::invalid_argument("unknown translation action");

	json::Object result;
	result["ok"] = true;
	result["available"] = current != available.end();
	result["boundary"] = boundary;
	result["exhausted"] = exhausted;
	result["count"] = static_cast<int64_t>(available.size());
	result["line_count"] = static_cast<int64_t>(context->ass->Events.size());
	if (current != available.end()) {
		context->selectionController->SetSelectionAndActive({current->line}, current->line);
		if (seek) context->videoController->JumpToTime(current->line->Start);
		result["id"] = current->line->Id;
		result["row"] = current->line->Row;
		result["block"] = static_cast<int64_t>(current->index);
		result["text"] = current->text;
		result["line_text"] = current->line->Text.get();
		result["position"] = static_cast<int64_t>(std::distance(available.begin(), current));
	}
	std::ostringstream stream;
	agi::JsonWriter::Write(result, stream);
	auto response = stream.str();
	response.erase(std::remove(response.begin(), response.end(), '\n'), response.end());
	return response;
}
}
