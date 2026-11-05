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

#pragma once

#include "localisation.h"

#include <libaegisub/format.h>

#include <string>
#include <utility>

namespace agi {
/// Format a message translated at run time; libaegisub's format only takes
/// string literals, but translations arrive as std::string.
template <typename... Args>
std::string format(std::string const& fmt, Args&&... args) {
	return format(fmt.c_str(), std::forward<Args>(args)...);
}
}

#define fmt_tl(str, ...) agi::format(_(str), __VA_ARGS__)
#define fmt_plural(n, sing, plural, ...) \
	agi::format(::agi::i18n::TranslatePlural(sing, plural, (n)), __VA_ARGS__)
