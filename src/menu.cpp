// Copyright (c) 2011, Thomas Goyne <plorkyeran@aegisub.org>
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

/// @file menu.cpp
/// @brief Menu configuration loader.

#include "include/aegisub/menu.h"

#include "options.h"
#include "libresrc/libresrc.h"

#include <libaegisub/cajun/reader.h>
#include <libaegisub/json.h>
#include <libaegisub/log.h>
#include <libaegisub/path.h>


namespace {
/// Get the root object of the menu configuration
json::Object const& get_menus_root() {
	static json::Object root;
	if (!root.empty()) return root;

	try {
		root = std::move(static_cast<json::Object&>(agi::json_util::file(config::path->Decode("?user/menu.json"), GET_DEFAULT_CONFIG(default_menu))));
		return root;
	}
	catch (json::Reader::ParseException const& e) {
		LOG_E("menu/parse") << "json::ParseException: " << e.what() << ", Line/offset: " << e.m_locTokenBegin.m_nLine + 1 << '/' << e.m_locTokenBegin.m_nLineOffset + 1;
		throw;
	}
	catch (std::exception const& e) {
		LOG_E("menu/parse") << e.what();
		throw;
	}
}
}

namespace menu {
json::Object const& GetConfig() {
	return get_menus_root();
}
}
