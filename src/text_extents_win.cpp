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

#include "text_extents.h"

#define WIN32_LEAN_AND_MEAN
#include <windows.h>

#include <libaegisub/charset_conv_win.h>

#include <cmath>
#include <string>

namespace agi::text {
bool MeasureExtents(std::string const& face, double fontsize, bool bold,
	bool italic, bool underline, bool strikeout, int encoding,
	std::string const& text, double spacing, Extents &extents)
{
	extents = {};

	auto dc = CreateCompatibleDC(nullptr);
	if (!dc) return false;

	SetMapMode(dc, MM_TEXT);

	LOGFONTW lf = {0};
	lf.lfHeight = (LONG)fontsize;
	lf.lfWeight = bold ? FW_BOLD : FW_NORMAL;
	lf.lfItalic = italic;
	lf.lfUnderline = underline;
	lf.lfStrikeOut = strikeout;
	lf.lfCharSet = encoding;
	lf.lfOutPrecision = OUT_TT_PRECIS;
	lf.lfClipPrecision = CLIP_DEFAULT_PRECIS;
	lf.lfQuality = ANTIALIASED_QUALITY;
	lf.lfPitchAndFamily = DEFAULT_PITCH | FF_DONTCARE;
	wcsncpy(lf.lfFaceName, agi::charset::ConvertW(face).c_str(), 31);

	auto font = CreateFontIndirect(&lf);
	if (!font) {
		DeleteDC(dc);
		return false;
	}

	auto old_font = SelectObject(dc, font);
	std::wstring wtext(agi::charset::ConvertW(text));
	if (spacing != 0) {
		for (auto c : wtext) {
			SIZE sz;
			GetTextExtentPoint32(dc, &c, 1, &sz);
			extents.width += sz.cx + spacing;
			extents.height = sz.cy;
		}
	}
	else {
		SIZE sz;
		GetTextExtentPoint32(dc, wtext.data(), (int)wtext.size(), &sz);
		extents.width = sz.cx;
		extents.height = sz.cy;
	}

	TEXTMETRIC tm;
	GetTextMetrics(dc, &tm);
	extents.descent = tm.tmDescent;
	extents.extlead = tm.tmExternalLeading;

	SelectObject(dc, old_font);
	DeleteObject(font);
	DeleteDC(dc);

	return true;
}
}
