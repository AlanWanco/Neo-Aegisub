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

#import <CoreText/CoreText.h>

#include <unicode/unistr.h>
#include <unicode/utf16.h>

namespace {
double line_width(CTFontRef font, CFStringRef text) {
	const void *keys[] = { kCTFontAttributeName };
	const void *values[] = { font };
	CFDictionaryRef attributes = CFDictionaryCreate(nullptr, keys, values, 1,
		&kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
	CFAttributedStringRef attributed = CFAttributedStringCreate(nullptr, text, attributes);
	CTLineRef line = CTLineCreateWithAttributedString(attributed);
	double width = CTLineGetTypographicBounds(line, nullptr, nullptr, nullptr);
	CFRelease(line);
	CFRelease(attributed);
	CFRelease(attributes);
	return width;
}

CFStringRef utf8_string(std::string const& text) {
	return CFStringCreateWithBytes(nullptr, reinterpret_cast<UInt8 const *>(text.data()),
		text.size(), kCFStringEncodingUTF8, false);
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

	CFStringRef name = utf8_string(face);
	if (!name) return false;
	CTFontRef font = CTFontCreateWithName(name, fontsize, nullptr);
	CFRelease(name);
	if (!font) return false;

	CTFontSymbolicTraits traits = 0;
	if (bold) traits |= kCTFontTraitBold;
	if (italic) traits |= kCTFontTraitItalic;
	if (traits) {
		if (CTFontRef styled = CTFontCreateCopyWithSymbolicTraits(font, fontsize, nullptr, traits, traits)) {
			CFRelease(font);
			font = styled;
		}
	}

	double ascent = CTFontGetAscent(font);
	double descent = CTFontGetDescent(font);
	extents.height = ascent + descent;
	extents.descent = descent;
	extents.extlead = CTFontGetLeading(font);

	if (spacing != 0) {
		icu::UnicodeString u16 = icu::UnicodeString::fromUTF8(text);
		for (int32_t i = 0; i < u16.length();) {
			UChar32 c = u16.char32At(i);
			i += U16_LENGTH(c);
			std::string utf8;
			icu::UnicodeString(c).toUTF8String(utf8);
			CFStringRef character = utf8_string(utf8);
			if (!character) continue;
			extents.width += line_width(font, character) + spacing;
			CFRelease(character);
		}
	}
	else {
		CFStringRef string = utf8_string(text);
		if (string) {
			extents.width = line_width(font, string);
			CFRelease(string);
		}
	}

	CFRelease(font);
	return true;
}
}
