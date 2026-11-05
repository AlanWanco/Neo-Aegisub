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

#include "clipboard.h"

#define WIN32_LEAN_AND_MEAN
#include <windows.h>

#include <libaegisub/charset_conv_win.h>

namespace clipboard {
std::string GetText() {
	std::string result;
	if (!OpenClipboard(nullptr)) return result;
	if (HANDLE data = GetClipboardData(CF_UNICODETEXT)) {
		if (auto text = static_cast<wchar_t const *>(GlobalLock(data))) {
			result = agi::charset::ConvertU(text);
			GlobalUnlock(data);
		}
	}
	CloseClipboard();
	return result;
}

void SetText(std::string const& text) {
	if (!OpenClipboard(nullptr)) return;
	EmptyClipboard();
	std::wstring wide = agi::charset::ConvertW(text);
	size_t bytes = (wide.size() + 1) * sizeof(wchar_t);
	if (HGLOBAL memory = GlobalAlloc(GMEM_MOVEABLE, bytes)) {
		if (void *target = GlobalLock(memory)) {
			memcpy(target, wide.c_str(), bytes);
			GlobalUnlock(memory);
			SetClipboardData(CF_UNICODETEXT, memory);
		}
		else {
			GlobalFree(memory);
		}
	}
	CloseClipboard();
}
}
