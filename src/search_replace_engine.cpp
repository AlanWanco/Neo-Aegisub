// Copyright (c) 2013, Thomas Goyne <plorkyeran@aegisub.org>
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
//
// Aegisub Project http://www.aegisub.org/

#include "search_replace_engine.h"
#include "utils.h"
#include "gpui_bridge.h"

#include "ass_dialogue.h"
#include "ass_file.h"
#include "format.h"
#include "include/aegisub/context.h"
#include "selection_controller.h"
#include "text_selection_controller.h"

#include <libaegisub/exception.h>
#include <libaegisub/util.h>

#include <boost/locale/conversion.hpp>

#include <unicode/normalizer2.h>
#include <unicode/edits.h>
#include <unicode/bytestream.h>
#include <limits>

namespace {
static const size_t bad_pos = -1;
static const MatchState bad_match{nullptr, 0, bad_pos, {}};

auto get_dialogue_field(SearchReplaceSettings::Field field) -> decltype(&AssDialogueBase::Text) {
	switch (field) {
		case SearchReplaceSettings::Field::TEXT: return &AssDialogueBase::Text;
		case SearchReplaceSettings::Field::STYLE: return &AssDialogueBase::Style;
		case SearchReplaceSettings::Field::ACTOR: return &AssDialogueBase::Actor;
		case SearchReplaceSettings::Field::EFFECT: return &AssDialogueBase::Effect;
	}
	throw agi::InternalError("Bad field for search");
}

typedef std::function<MatchState (const AssDialogue*, size_t)> matcher;

class normalized_accessor {
	boost::flyweight<std::string> AssDialogueBase::*field;
	bool skip_tags;
	icu::Edits edits;
	agi::util::tagless_find_helper helper;

public:
	size_t position = 0;
	normalized_accessor(SearchReplaceSettings::Field field, bool skip_tags)
	: field(get_dialogue_field(field)), skip_tags(skip_tags) { }

	std::string get(AssDialogue const* line, size_t offset) {
		auto const& raw = (line->*field).get();
		if (raw.size() > static_cast<size_t>(std::numeric_limits<int32_t>::max()) || offset > raw.size())
			throw agi::InvalidInputException("Search position is outside the text");
		std::string normalized;
		icu::StringByteSink sink(&normalized);
		UErrorCode error = U_ZERO_ERROR;
		auto normalizer = icu::Normalizer2::getNFCInstance(error);
		if (U_FAILURE(error)) throw agi::InvalidInputException(u_errorName(error));
		normalizer->normalizeUTF8(0, icu::StringPiece(raw.data(), static_cast<int32_t>(raw.size())), sink, &edits, error);
		position = edits.getFineIterator().destinationIndexFromSourceIndex(static_cast<int32_t>(offset), error);
		if (U_FAILURE(error)) throw agi::InvalidInputException(u_errorName(error));
		if (!skip_tags) return normalized;
		auto text = helper.strip_tags(normalized, 0);
		agi::util::tagless_find_helper suffix;
		position = text.size() - suffix.strip_tags(normalized, position).size();
		return text;
	}

	MatchState make_match_state(size_t begin, size_t end, boost::u32regex *regex = nullptr) {
		if (skip_tags) helper.map_range(begin, end);
		UErrorCode error = U_ZERO_ERROR;
		auto mapping = edits.getFineIterator();
		begin = mapping.sourceIndexFromDestinationIndex(static_cast<int32_t>(begin), error);
		end = mapping.sourceIndexFromDestinationIndex(static_cast<int32_t>(end), error);
		if (U_FAILURE(error)) throw agi::InvalidInputException(u_errorName(error));
		return {regex, begin, end, {}};
	}
};

template<typename Accessor>
matcher get_matcher(SearchReplaceSettings const& settings, Accessor&& a) {
	if (settings.use_regex) {
		int flags = boost::u32regex::perl;
		if (!settings.match_case)
			flags |= boost::u32regex::icase;

		auto regex = boost::make_u32regex(settings.find, flags);

		return [=](const AssDialogue *diag, size_t start) mutable -> MatchState {
			boost::smatch result;
			auto const& str = a.get(diag, start);
			auto flags = a.position ? boost::match_not_bol | boost::match_prev_avail : boost::match_default;
			if (!u32regex_search(str.begin() + a.position, str.end(), result, regex, flags, str.begin()))
				return bad_match;
			auto const begin = std::distance(str.begin(), result[0].first);
			auto const end = std::distance(str.begin(), result[0].second);
			auto match = a.make_match_state(begin, end, &regex);
			match.replacement = result.format(settings.replace_with);
			return match;
		};
	}

	bool full_match_only = settings.exact_match;
	bool match_case = settings.match_case;
	std::string look_for = settings.find;

	if (!settings.match_case)
		look_for = boost::locale::fold_case(look_for);

	return [=](const AssDialogue *diag, size_t start) mutable -> MatchState {
		const auto full = a.get(diag, start);
		const auto str = full.substr(a.position);
		if (full_match_only && str.size() != look_for.size())
			return bad_match;

		if (match_case) {
			const auto pos = str.find(look_for);
			return pos == std::string::npos ? bad_match : a.make_match_state(pos + a.position, pos + a.position + look_for.size());
		}

		const auto pos = agi::util::ifind(str, look_for);
		return pos.first == bad_pos ? bad_match : a.make_match_state(pos.first + a.position, pos.second + a.position);
	};
}

template<typename Iterator, typename Container>
Iterator circular_next(Iterator it, Container& c) {
	++it;
	if (it == c.end())
		it = c.begin();
	return it;
}

}

std::function<MatchState (const AssDialogue*, size_t)> SearchReplaceEngine::GetMatcher(SearchReplaceSettings const& settings) {
	auto normalized = settings;
	normalized.find = boost::locale::normalize(normalized.find);
	return get_matcher(normalized, normalized_accessor(settings.field, settings.skip_tags));
}

SearchReplaceEngine::SearchReplaceEngine(agi::Context *c)
: context(c)
{
}

void SearchReplaceEngine::ApplyReplacement(AssDialogue *diag, MatchState &ms, SearchReplaceSettings const& settings) {
	auto& diag_field = diag->*get_dialogue_field(settings.field);
	auto text = diag_field.get();
	std::string const& replacement = ms.re ? ms.replacement : settings.replace_with;
	diag_field = text.substr(0, ms.start) + replacement + text.substr(ms.end);
	ms.end = ms.start + replacement.size();
}

void SearchReplaceEngine::Replace(AssDialogue *diag, MatchState &ms) {
	ApplyReplacement(diag, ms, settings);
}

bool SearchReplaceEngine::FindReplace(bool replace) {
	if (!initialized)
		return false;

	auto matches = GetMatcher(settings);

	AssDialogue *line = context->selectionController->GetActiveLine();
	auto it = context->ass->iterator_to(*line);
	size_t pos = 0;

	auto replace_ms = bad_match;
	if (replace) {
		if (settings.field == SearchReplaceSettings::Field::TEXT)
			pos = context->textSelectionController->GetSelectionStart();

		if ((replace_ms = matches(line, pos))) {
			size_t end = bad_pos;
			if (settings.field == SearchReplaceSettings::Field::TEXT)
				end = context->textSelectionController->GetSelectionEnd();

			if (end == bad_pos || (pos == replace_ms.start && end == replace_ms.end)) {
				Replace(line, replace_ms);
				pos = replace_ms.end;
				context->ass->Commit(_("replace"), AssFile::COMMIT_DIAG_TEXT);
			}
			else {
				// The current line matches, but it wasn't already selected,
				// so the match hasn't been "found" and displayed to the user
				// yet, so do that rather than replacing
				context->textSelectionController->SetSelection(replace_ms.start, replace_ms.end);
				return true;
			}
		}
	}
	// Search from the end of the selection to avoid endless matching the same thing
	else if (settings.field == SearchReplaceSettings::Field::TEXT)
		pos = context->textSelectionController->GetSelectionEnd();
	// For non-text fields we just look for matching lines rather than each
	// match within the line, so move to the next line
	else if (settings.field != SearchReplaceSettings::Field::TEXT)
		it = circular_next(it, context->ass->Events);

	auto const& sel = context->selectionController->GetSelectedSet();
	bool selection_only = sel.size() > 1 && settings.limit_to == SearchReplaceSettings::Limit::SELECTED;

	do {
		if (selection_only && !sel.count(&*it)) continue;
		if (settings.ignore_comments && it->Comment) continue;

		if (MatchState ms = matches(&*it, pos)) {
			if (selection_only)
				// We're cycling through the selection, so don't muck with it
				context->selectionController->SetActiveLine(&*it);
			else
				context->selectionController->SetSelectionAndActive({ &*it }, &*it);

			if (settings.field == SearchReplaceSettings::Field::TEXT)
				context->textSelectionController->SetSelection(ms.start, ms.end);

			return true;
		}
	} while (pos = 0, &*(it = circular_next(it, context->ass->Events)) != line);

	// Replaced something and didn't find another match, so select the newly
	// inserted text
	if (replace_ms && settings.field == SearchReplaceSettings::Field::TEXT)
		context->textSelectionController->SetSelection(replace_ms.start, replace_ms.end);

	return true;
}

size_t SearchReplaceEngine::ReplaceAllMatches() {
	if (!initialized || settings.find.empty()) return 0;
	size_t count = 0;
	auto matches = GetMatcher(settings);
	auto const& selection = context->selectionController->GetSelectedSet();
	auto field = get_dialogue_field(settings.field);
	std::vector<std::pair<AssDialogue *, std::string>> changes;
	for (auto& line : context->ass->Events) {
		if (settings.limit_to == SearchReplaceSettings::Limit::SELECTED && !selection.count(&line)) continue;
		if (settings.ignore_comments && line.Comment) continue;
		auto const& original = (line.*field).get();
		std::string replacement;
		size_t position = 0, previous = 0;
		while (MatchState match = matches(&line, position)) {
			++count;
			replacement.append(original, previous, match.start - previous);
			replacement += match.re ? match.replacement : settings.replace_with;
			previous = match.end;
			position = match.end;
			if (match.start == match.end) {
				if (position >= original.size()) break;
				do { ++position; } while (position < original.size() && (static_cast<unsigned char>(original[position]) & 0xc0) == 0x80);
			}
		}
		replacement.append(original, previous, std::string::npos);
		if (replacement != original) changes.emplace_back(&line, std::move(replacement));
	}
	for (auto& [line, value] : changes) line->*field = std::move(value);
	if (!changes.empty()) context->ass->Commit(_("replace"), settings.field == SearchReplaceSettings::Field::TEXT ? AssFile::COMMIT_DIAG_TEXT : AssFile::COMMIT_DIAG_META);
	return count;
}

bool SearchReplaceEngine::ReplaceAll() {
	if (!initialized) return false;
	auto count = ReplaceAllMatches();
	if (gpui::FrontendAttached()) {
		if (count)
			gpui::PushNotice(_("Replace"), fmt_plural(count, "One match was replaced.", "%d matches were replaced.", static_cast<int>(count)));
		else
			gpui::PushNotice(_("Replace"), _("No matches found."));
		return true;
	}
	if (count) gpui::PushStatus(fmt_plural(count, "One match was replaced.", "%d matches were replaced.", static_cast<int>(count)));
	else gpui::PushStatus(_("No matches found."));
	return true;
}

void SearchReplaceEngine::Configure(SearchReplaceSettings const& new_settings) {
	settings = new_settings;
	initialized = true;
}
