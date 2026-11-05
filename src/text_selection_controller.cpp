// Copyright (c) 2014, Thomas Goyne <plorkyeran@aegisub.org>
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

#include "text_selection_controller.h"

void TextSelectionController::SetInsertionPoint(int position) {
	if (insertion_point == position) return;
	insertion_point = position;
	if (selection_start == selection_end) {
		selection_start = position;
		selection_end = position;
	}
	AnnounceSelectionChanged();
}

void TextSelectionController::SetSelection(int start, int end) {
	if (start > end) std::swap(start, end);
	if (start == selection_start && end == selection_end) return;
	selection_start = start;
	selection_end = end;
	insertion_point = end;
	AnnounceSelectionChanged();
}
