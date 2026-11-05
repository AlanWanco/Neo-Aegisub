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

#include <fontconfig/fontconfig.h>
#include <ft2build.h>
#include FT_FREETYPE_H
#include FT_ADVANCES_H

#include <unicode/unistr.h>
#include <unicode/utf16.h>

#include <cmath>
#include <mutex>

namespace {
struct Freetype {
	FT_Library library = nullptr;
	Freetype() { FT_Init_FreeType(&library); }
	~Freetype() { if (library) FT_Done_FreeType(library); }
};

FT_Library freetype() {
	static Freetype instance;
	return instance.library;
}

FT_Face open_face(std::string const& family, bool bold, bool italic) {
	FcPattern *pattern = FcPatternCreate();
	if (!pattern) return nullptr;
	FcPatternAddString(pattern, FC_FAMILY, reinterpret_cast<FcChar8 const *>(family.c_str()));
	FcPatternAddInteger(pattern, FC_WEIGHT, bold ? FC_WEIGHT_BOLD : FC_WEIGHT_REGULAR);
	FcPatternAddInteger(pattern, FC_SLANT, italic ? FC_SLANT_ITALIC : FC_SLANT_ROMAN);
	FcConfigSubstitute(nullptr, pattern, FcMatchPattern);
	FcDefaultSubstitute(pattern);

	FcResult result;
	FcPattern *match = FcFontMatch(nullptr, pattern, &result);
	FcPatternDestroy(pattern);
	if (!match) return nullptr;

	FcChar8 *file = nullptr;
	int index = 0;
	if (FcPatternGetString(match, FC_FILE, 0, &file) != FcResultMatch) {
		FcPatternDestroy(match);
		return nullptr;
	}
	FcPatternGetInteger(match, FC_INDEX, 0, &index);

	FT_Face face = nullptr;
	if (!freetype() || FT_New_Face(freetype(), reinterpret_cast<char const *>(file), index, &face))
		face = nullptr;
	FcPatternDestroy(match);
	return face;
}
}

namespace agi::text {
bool MeasureExtents(std::string const& face, double fontsize, bool bold,
	bool italic, bool underline, bool strikeout, int encoding,
	std::string const& text, double spacing, Extents &extents)
{
	(void)underline;
	(void)strikeout;
	(void)encoding;
	extents = {};

	static std::mutex mutex;
	std::lock_guard<std::mutex> lock(mutex);

	FT_Face ft_face = open_face(face, bold, italic);
	if (!ft_face) return false;

	int pixel_size = std::max(1, (int)std::lround(fontsize));
	if (FT_Set_Pixel_Sizes(ft_face, 0, pixel_size)) {
		FT_Done_Face(ft_face);
		return false;
	}

	extents.height = ft_face->size->metrics.ascender - ft_face->size->metrics.descender;
	extents.descent = -ft_face->size->metrics.descender;
	extents.extlead = 0;

	icu::UnicodeString content = icu::UnicodeString::fromUTF8(text);
	FT_UInt previous = 0;
	for (int32_t i = 0; i < content.length();) {
		UChar32 c = content.char32At(i);
		i += U16_LENGTH(c);
		FT_UInt glyph = FT_Get_Char_Index(ft_face, (FT_ULong)c);
		if (FT_Load_Glyph(ft_face, glyph, FT_LOAD_DEFAULT))
			continue;
		if (previous && FT_HAS_KERNING(ft_face)) {
			FT_Vector kerning;
			FT_Get_Kerning(ft_face, previous, glyph, FT_KERNING_UNFITTED, &kerning);
			extents.width += kerning.x;
		}
		extents.width += ft_face->glyph->advance.x + spacing;
		previous = glyph;
	}

	FT_Done_Face(ft_face);
	return true;
}
}
