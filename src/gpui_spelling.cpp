// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

#include "gpui_spelling.h"
#include "localisation.h"

#include "ass_dialogue.h"
#include "ass_file.h"
#include "include/aegisub/context.h"
#include "include/aegisub/spellchecker.h"
#include "options.h"
#include "selection_controller.h"
#include "text_selection_controller.h"

#include <libaegisub/ass/dialogue_parser.h>
#include <libaegisub/cajun/writer.h>
#include <libaegisub/spellchecker.h>

#include <algorithm>
#include <boost/locale/conversion.hpp>
#include <optional>
#include <set>
#include <sstream>
#include <stdexcept>
#include <tuple>

namespace gpui {
struct Spelling::Impl {
	struct Word { AssDialogue *line; size_t start, length; std::string text; };
	struct Match { int id; size_t start, length; std::string text, line_text; };
	agi::Context *context;
	std::unique_ptr<agi::SpellChecker> checker;
	std::set<std::string> ignored;
	std::set<std::tuple<int, size_t, std::string>> skipped;
	std::optional<Match> current;
	bool complete = false;

	explicit Impl(agi::Context *context) : context(context) { }

	AssDialogue *Line(int id) {
		auto it = std::find_if(context->ass->Events.begin(), context->ass->Events.end(), [&](auto const& line) { return line.Id == id; });
		return it == context->ass->Events.end() ? nullptr : &*it;
	}

	void Initialize() {
		if (!checker) checker = SpellCheckerFactory::GetSpellChecker();
		if (!checker) return;
		auto languages = checker->GetLanguageList();
		auto language = OPT_GET("Tool/Spell Checker/Language")->GetString();
		if (!languages.empty() && std::find(languages.begin(), languages.end(), language) == languages.end()) {
			auto selected = std::find(languages.begin(), languages.end(), "en_US");
			if (selected == languages.end()) selected = std::find(languages.begin(), languages.end(), "en");
			OPT_SET("Tool/Spell Checker/Language")->SetString(selected == languages.end() ? languages.front() : *selected);
		}
	}

	std::vector<Word> Words() {
		std::vector<Word> words;
		for (auto& line : context->ass->Events) {
			if (line.Comment && OPT_GET("Tool/Spell Checker/Skip Comments")->GetBool()) continue;
			auto const& text = line.Text.get();
			auto tokens = agi::ass::TokenizeDialogueBody(text);
			agi::ass::SplitWords(text, tokens);
			size_t offset = 0;
			for (auto const& token : tokens) {
				if (token.type == agi::ass::DialogueTokenType::WORD) {
					auto word = text.substr(offset, token.length);
					if (!(OPT_GET("Tool/Spell Checker/Skip Uppercase")->GetBool() && word == boost::locale::to_upper(word)))
						words.push_back({&line, offset, token.length, std::move(word)});
				}
				offset += token.length;
			}
		}
		return words;
	}

	void Find(int row, size_t offset) {
		current.reset(); complete = true;
		auto words = Words();
		auto first = std::find_if(words.begin(), words.end(), [&](auto const& word) { return word.line->Row > row || (word.line->Row == row && word.start >= offset); });
		auto check = [&](auto begin, auto end) {
			for (; begin != end; ++begin) {
				auto const& word = *begin;
				if (ignored.count(word.text) || skipped.count({word.line->Id, word.start, word.line->Text.get()}) || checker->CheckWord(word.text)) continue;
				current = Match{word.line->Id, word.start, word.length, word.text, word.line->Text.get()};
				complete = false;
				context->selectionController->SetSelectionAndActive({word.line}, word.line);
				context->textSelectionController->SetSelection(word.start, word.start + word.length);
				return true;
			}
			return false;
		};
		if (!check(first, words.end())) check(words.begin(), first);
	}

	AssDialogue *Validate(json::Object const& request) {
		if (!current) throw std::invalid_argument("start a spelling check before editing a word");
		auto line = Line(current->id);
		int64_t id = static_cast<json::Integer const&>(request.at("id"));
		int64_t start = static_cast<json::Integer const&>(request.at("start"));
		std::string const& expected = request.at("expected");
		if (!line || id != current->id || start < 0 || static_cast<size_t>(start) != current->start || expected != current->text || line->Text.get() != current->line_text)
			throw std::invalid_argument("subtitle text changed; restart the spelling check");
		return line;
	}
};

Spelling::Spelling(agi::Context *context) : impl(std::make_unique<Impl>(context)) { }
Spelling::~Spelling() = default;

std::string Spelling::Request(json::Object const& request) {
	impl->Initialize();
	auto context = impl->context;
	auto checker = impl->checker.get();
	std::string const& action = request.at("action");
	if (action != "state" && (!checker || checker->GetLanguageList().empty())) throw std::invalid_argument("no spelling dictionary is available");
	int changed = 0;
	if (action == "configure") {
		std::string const& language = request.at("language");
		auto languages = checker->GetLanguageList();
		if (std::find(languages.begin(), languages.end(), language) == languages.end()) throw std::invalid_argument("choose an available spelling language");
		bool comments = static_cast<json::Boolean const&>(request.at("skip_comments")), uppercase = static_cast<json::Boolean const&>(request.at("skip_uppercase"));
		OPT_SET("Tool/Spell Checker/Language")->SetString(language);
		OPT_SET("Tool/Spell Checker/Skip Comments")->SetBool(comments);
		OPT_SET("Tool/Spell Checker/Skip Uppercase")->SetBool(uppercase);
		impl->ignored.clear(); impl->skipped.clear(); impl->current.reset(); impl->complete = false;
	}
	else if (action == "start") {
		impl->ignored.clear(); impl->skipped.clear();
		auto active = context->selectionController->GetActiveLine();
		impl->Find(active ? active->Row : 0, 0);
	}
	else if (action == "next" || action == "ignore_all" || action == "replace" || action == "replace_all" || action == "add") {
		auto line = impl->Validate(request);
		auto current = *impl->current;
		size_t next = current.start + current.length;
		if (action == "next") impl->skipped.emplace(current.id, current.start, current.line_text);
		else if (action == "ignore_all") impl->ignored.insert(current.text);
		else if (action == "add") {
			if (!checker->CanAddWord(current.text)) throw std::invalid_argument("this word cannot be added to the dictionary");
			checker->AddWord(current.text);
		}
		else if (action == "replace" || action == "replace_all") {
			std::string const& replacement = request.at("replacement");
			if (replacement.find_first_of("\r\n") != replacement.npos) throw std::invalid_argument("spelling replacements must fit on one line");
			std::vector<std::pair<AssDialogue *, std::string>> patches;
			if (action == "replace") {
				auto text = line->Text.get(); text.replace(current.start, current.length, replacement);
				if (text != line->Text.get()) { patches.emplace_back(line, std::move(text)); ++changed; }
			}
			else {
				auto words = impl->Words();
				for (auto it = words.rbegin(); it != words.rend(); ++it) {
					if (it->text != current.text || replacement == current.text) continue;
					auto patch = std::find_if(patches.begin(), patches.end(), [&](auto const& patch) { return patch.first == it->line; });
					if (patch == patches.end()) { patches.emplace_back(it->line, it->line->Text.get()); patch = std::prev(patches.end()); }
					patch->second.replace(it->start, it->length, replacement); ++changed;
				}
			}
			for (auto& [target, text] : patches) target->Text = std::move(text);
			if (changed) context->ass->Commit(_("spell check replace"), AssFile::COMMIT_DIAG_TEXT);
			impl->skipped.emplace(current.id, current.start, line->Text.get());
			next = current.start + replacement.size();
		}
		// Recompute tokens after each action, so inserted ASS tags and changed
		// UTF-8 lengths cannot leave the next word at an obsolete byte offset.
		impl->Find(line->Row, next);
	}
	else if (action == "remove") {
		std::string const& word = request.at("word");
		if (word.empty() || !checker->CanRemoveWord(word)) throw std::invalid_argument("the word is not in the personal dictionary");
		checker->RemoveWord(word);
		impl->current.reset(); impl->complete = false;
	}
	else if (action != "state") throw std::invalid_argument("unknown spelling action");

	json::Object result;
	result["ok"] = true;
	result["available"] = checker && !checker->GetLanguageList().empty();
	result["language"] = OPT_GET("Tool/Spell Checker/Language")->GetString();
	result["skip_comments"] = OPT_GET("Tool/Spell Checker/Skip Comments")->GetBool();
	result["skip_uppercase"] = OPT_GET("Tool/Spell Checker/Skip Uppercase")->GetBool();
	result["complete"] = impl->complete;
	result["changed"] = changed;
	json::Array languages;
	if (checker) for (auto const& language : checker->GetLanguageList()) languages.emplace_back(language);
	result["languages"] = std::move(languages);
	bool found = impl->current && impl->Line(impl->current->id) && impl->Line(impl->current->id)->Text.get() == impl->current->line_text;
	result["found"] = found;
	if (found) {
		auto const& word = *impl->current;
		result["id"] = word.id; result["row"] = impl->Line(word.id)->Row;
		result["start"] = static_cast<int64_t>(word.start);
		result["length"] = static_cast<int64_t>(word.length);
		result["word"] = word.text; result["line_text"] = word.line_text;
		result["can_add"] = checker->CanAddWord(word.text);
		json::Array suggestions;
		for (auto const& suggestion : checker->GetSuggestions(word.text)) suggestions.emplace_back(suggestion);
		result["suggestions"] = std::move(suggestions);
	}
	if (auto word = request.find("word"); word != request.end() && checker) {
		std::string const& candidate = word->second;
		result["dictionary_word"] = candidate;
		result["can_remove"] = checker->CanRemoveWord(candidate);
	}
	std::ostringstream stream;
	agi::JsonWriter::Write(result, stream);
	auto response = stream.str(); response.erase(std::remove(response.begin(), response.end(), '\n'), response.end());
	return response;
}
}
