// Copyright (c) 2005, Rodrigo Braz Monteiro
// All rights reserved.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are met:
//
//   * Redistributions of source code must retain the above copyright notice,
//     this list of conditions and the following disclaimer.
//   * Redistributions in binary form must reproduce the above copyright notice,
//     this list of conditions and the following disclaimer in the documentation
//     and/or other materials provided with the distribution.
//   * Neither the name of the Aegisub Group nor the names of its contributors
//     may be used to endorse or promote products derived from this software
//     without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
// AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
// IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
// ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE
// LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
// CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
// SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
// INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
// CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
// ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
// POSSIBILITY OF SUCH DAMAGE.
//
// Aegisub Project http://www.aegisub.org/

/// @file aegisublocale.cpp
/// @brief Enumerate available locales for picking translation on Windows
/// @ingroup utility
///

#include "aegisublocale.h"

#include "compat.h"
#include "language_match.h"
#include "options.h"
#include "utils.h"

#include <libaegisub/path.h>

#include <algorithm>
#include <clocale>
#include <cstdio>
#include <cstdlib>
#include <functional>
#include <wx/intl.h>
#include <wx/stdpaths.h>

#ifndef AEGISUB_CATALOG
#define AEGISUB_CATALOG "aegisub"
#endif

wxTranslations *AegisubLocale::GetTranslations() {
	wxTranslations *translations = wxTranslations::Get();
	if (!translations) {
		wxTranslations::Set(translations = new wxTranslations);
		// A dev build keeps its compiled catalogs in the build tree, so let the
		// launcher point at them; without this every wx string falls back to
		// English while the GPUI workspace is Chinese.
		if (char const *override_dir = std::getenv("AEGISUB_LOCALE_DIR")) {
			if (*override_dir) {
				wxFileTranslationsLoader::AddCatalogLookupPathPrefix(to_wx(override_dir));
			}
		}
		wxFileTranslationsLoader::AddCatalogLookupPathPrefix(config::path->Decode("?data/locale/").wstring());
#if !defined(_WIN32) && !defined(__APPLE__) && !defined(APPIMAGE_BUILD)
		wxFileTranslationsLoader::AddCatalogLookupPathPrefix(P_LOCALE);
#endif
#if defined(__APPLE__)
		// A .app bundle installs its catalogs under Contents/Resources, which is
		// neither ?data/locale nor the P_LOCALE prefix.
		wxFileTranslationsLoader::AddCatalogLookupPathPrefix(
			wxStandardPaths::Get().GetResourcesDir());
#endif
	}
	return translations;
}

void AegisubLocale::Init(std::string const& language) {
	wxTranslations *translations = GetTranslations();
	translations->SetLanguage(to_wx(language));
	// wx searches the catalogs in reverse order of addition, so the standard
	// one has to go first: its "Light" (a font weight) would otherwise win
	// over ours (the light theme).
	translations->AddStdCatalog();
	bool loaded = translations->AddCatalog(AEGISUB_CATALOG);
	std::fprintf(stderr, "[locale] %s catalog %s\n", language.c_str(), loaded ? "loaded" : "MISSING");

	setlocale(LC_NUMERIC, "C");
	setlocale(LC_CTYPE, "C");
	active_language = language;
}

bool AegisubLocale::HasLanguage(std::string const& language) {
	auto langs = GetTranslations()->GetAvailableTranslations(AEGISUB_CATALOG);
	return std::find(langs.begin(), langs.end(), to_wx(language)) != langs.end();
}

std::vector<std::pair<std::string, std::string>> AegisubLocale::GetAvailableLanguages() {
	wxArrayString codes = GetTranslations()->GetAvailableTranslations(AEGISUB_CATALOG);
	std::vector<std::pair<std::string, std::string>> languages;
	languages.emplace_back("en_US", from_wx(LocalizedLanguageName("en_US")));
	for (auto const& code : codes) {
		auto language = from_wx(code);
		if (language == "en_US") continue;
		languages.emplace_back(language, from_wx(LocalizedLanguageName(code)));
	}
	return languages;
}

std::string AegisubLocale::RecommendLanguage() {
	auto codes = GetTranslations()->GetAvailableTranslations(AEGISUB_CATALOG);
	wxString os_ui_language = GetTranslations()->GetBestTranslation(AEGISUB_CATALOG);
	if (!os_ui_language.empty())
		return from_wx(os_ui_language);

	const int system_language = wxLocale::GetSystemLanguage();
	if (system_language != wxLANGUAGE_UNKNOWN) {
		const wxLanguageInfo *system_info = wxLocale::GetLanguageInfo(system_language);
		for (auto const& code : codes) {
			const wxLanguageInfo *info = wxLocale::FindLanguageInfo(code);
			if (system_info && info && LanguagesMatch(*system_info, *info))
				return from_wx(code);
		}
	}

	return "en_US";
}
