// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

#include "grid_display_text.h"

#include "ass_dialogue.h"
#include "options.h"

#include <libaegisub/option.h>

std::string GridDisplayText(AssDialogue const& line) {
	std::string text;
	auto const& raw = line.Text.get();
	int const mode = OPT_GET("Subtitle/Grid/Hide Overrides")->GetInt();
	if (!mode) text = raw;
	else {
		size_t start = 0;
		while (start < raw.size()) {
			auto open = raw.find('{', start);
			if (open == raw.npos) { text.append(raw, start, raw.npos); break; }
			text.append(raw, start, open - start);
			if (mode == 1) text += OPT_GET("Subtitle/Grid/Hide Overrides Char")->GetString();
			auto close = raw.find('}', open);
			if (close == raw.npos) break;
			start = close + 1;
		}
	}
	if (text.size() > 512) text = text.substr(0, 512) + "...";
	return text;
}
