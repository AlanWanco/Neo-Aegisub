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
//
// Aegisub Project http://www.aegisub.org/

/// @file localisation.cpp
/// @brief Minimal gettext catalog reader used by the headless host and its
///        command layer; replaces wxTranslations and wxLocale.
/// @ingroup utility

#include "localisation.h"

#include "options.h"
#include "platform_path.h"

#include <libaegisub/path.h>

#include <algorithm>
#include <cctype>
#include <clocale>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <memory>
#include <stdexcept>
#include <string_view>
#include <unordered_map>

#include <unicode/locid.h>
#include <unicode/unistr.h>

#if defined(__APPLE__)
#include <CoreFoundation/CFLocale.h>
#include <CoreFoundation/CFString.h>
#elif defined(_WIN32)
#include <windows.h>
#else
#include <cstdlib>
#endif

#ifndef AEGISUB_CATALOG
#define AEGISUB_CATALOG "aegisub"
#endif

namespace {
namespace fsp = std::filesystem;

std::string const &CatalogName() {
	static std::string const name = AEGISUB_CATALOG;
	return name;
}

std::uint32_t ReadU32(std::string_view data, size_t offset) {
	if (offset + 4 > data.size()) return 0;
	unsigned char const *p = reinterpret_cast<unsigned char const *>(data.data() + offset);
	return static_cast<std::uint32_t>(p[0]) | (static_cast<std::uint32_t>(p[1]) << 8) |
	       (static_cast<std::uint32_t>(p[2]) << 16) | (static_cast<std::uint32_t>(p[3]) << 24);
}

/// A plural form expression from a catalog header, evaluated per count.
class PluralRule {
public:
	int index(int n) const {
		if (expression.empty()) return n == 1 ? 0 : 1;
		Program const &program = Compile();
		int position = 0;
		long long value = Evaluate(program, position, n);
		if (value < 0) value = 0;
		if (value >= static_cast<long long>(forms)) value = forms - 1;
		return static_cast<int>(value);
	}

	void Configure(std::string const &header) {
		auto field = header.find("Plural-Forms:");
		if (field == std::string::npos) return;
		auto end = header.find('\n', field);
		std::string line = header.substr(field, end == std::string::npos ? std::string::npos : end - field);
		auto nplurals = line.find("nplurals=");
		if (nplurals != std::string::npos)
			forms = std::max(1L, std::strtol(line.c_str() + nplurals + 9, nullptr, 10));
		auto plural = line.find("plural=");
		if (plural != std::string::npos) {
			expression = line.substr(plural + 7);
			while (!expression.empty() && (expression.back() == ';' || std::isspace(static_cast<unsigned char>(expression.back()))))
				expression.pop_back();
			compiled_program = Program{};
		}
	}

	int forms = 2;
	std::string expression;

private:
	enum class Op { Constant, N, Add, Sub, Mul, Div, Mod, Eq, Ne, Lt, Le, Gt, Ge, And, Or, Not, Ternary };
	struct Instruction {
		Op op;
		long long value = 0;
	};
	using Program = std::vector<Instruction>;

	struct Parser {
		std::string_view text;
		size_t position = 0;
		Program program;

		void Skip() {
			while (position < text.size() && std::isspace(static_cast<unsigned char>(text[position]))) ++position;
		}

		bool Accept(std::string_view token) {
			Skip();
			if (text.substr(position, token.size()) != token) return false;
			position += token.size();
			return true;
		}

		bool Peek(char c) {
			Skip();
			return position < text.size() && text[position] == c;
		}

		void Emit(Op op, long long value = 0) { program.push_back(Instruction{op, value}); }

		void ParseTernary() {
			ParseOr();
			if (Accept("?")) {
				ParseTernary();
				if (!Accept(":")) throw std::runtime_error("plural rule: missing ':'");
				ParseTernary();
				Emit(Op::Ternary);
			}
		}

		void ParseOr() {
			ParseAnd();
			while (Accept("||")) {
				ParseAnd();
				Emit(Op::Or);
			}
		}

		void ParseAnd() {
			ParseCompare();
			while (Accept("&&")) {
				ParseCompare();
				Emit(Op::And);
			}
		}

		void ParseCompare() {
			ParseAdd();
			for (;;) {
				if (Accept("==")) { ParseAdd(); Emit(Op::Eq); }
				else if (Accept("!=")) { ParseAdd(); Emit(Op::Ne); }
				else if (Accept("<=")) { ParseAdd(); Emit(Op::Le); }
				else if (Accept(">=")) { ParseAdd(); Emit(Op::Ge); }
				else if (Accept("<")) { ParseAdd(); Emit(Op::Lt); }
				else if (Accept(">")) { ParseAdd(); Emit(Op::Gt); }
				else return;
			}
		}

		void ParseAdd() {
			ParseMul();
			for (;;) {
				if (Accept("+")) { ParseMul(); Emit(Op::Add); }
				else if (Accept("-")) { ParseMul(); Emit(Op::Sub); }
				else return;
			}
		}

		void ParseMul() {
			ParseUnary();
			for (;;) {
				if (Accept("*")) { ParseUnary(); Emit(Op::Mul); }
				else if (Accept("/")) { ParseUnary(); Emit(Op::Div); }
				else if (Accept("%")) { ParseUnary(); Emit(Op::Mod); }
				else return;
			}
		}

		void ParseUnary() {
			if (Accept("!")) { ParseUnary(); Emit(Op::Not); return; }
			if (Accept("(")) {
				ParseTernary();
				if (!Accept(")")) throw std::runtime_error("plural rule: missing ')'");
				return;
			}
			Skip();
			if (position < text.size() && text[position] == 'n') {
				++position;
				Emit(Op::N);
				return;
			}
			char *end = nullptr;
			long long value = std::strtoll(text.data() + position, &end, 10);
			if (end == text.data() + position) throw std::runtime_error("plural rule: unexpected token");
			position = static_cast<size_t>(end - text.data());
			Emit(Op::Constant, value);
		}
	};

	mutable Program compiled_program;

	Program const &Compile() const {
		if (!compiled_program.empty() || expression.empty()) return compiled_program;
		Parser parser{expression};
		try {
			parser.ParseTernary();
			parser.Skip();
			if (parser.position != expression.size()) throw std::runtime_error("plural rule: trailing input");
			compiled_program = std::move(parser.program);
		}
		catch (...) {
			compiled_program.clear();
		}
		return compiled_program;
	}

	static long long Evaluate(Program const &program, int &position, long long n) {
		if (position >= static_cast<int>(program.size())) return 0;
		Instruction const instruction = program[position++];
		switch (instruction.op) {
		case Op::Constant: return instruction.value;
		case Op::N: return n;
		case Op::Add: return Evaluate(program, position, n) + Evaluate(program, position, n);
		case Op::Sub: return Evaluate(program, position, n) - Evaluate(program, position, n);
		case Op::Mul: return Evaluate(program, position, n) * Evaluate(program, position, n);
		case Op::Div: {
			long long right = Evaluate(program, position, n);
			return right ? Evaluate(program, position, n) / right : 0;
		}
		case Op::Mod: {
			long long right = Evaluate(program, position, n);
			return right ? Evaluate(program, position, n) % right : 0;
		}
		case Op::Eq: return Evaluate(program, position, n) == Evaluate(program, position, n);
		case Op::Ne: return Evaluate(program, position, n) != Evaluate(program, position, n);
		case Op::Lt: return Evaluate(program, position, n) < Evaluate(program, position, n);
		case Op::Le: return Evaluate(program, position, n) <= Evaluate(program, position, n);
		case Op::Gt: return Evaluate(program, position, n) > Evaluate(program, position, n);
		case Op::Ge: return Evaluate(program, position, n) >= Evaluate(program, position, n);
		case Op::And: return Evaluate(program, position, n) && Evaluate(program, position, n);
		case Op::Or: return Evaluate(program, position, n) || Evaluate(program, position, n);
		case Op::Not: return !Evaluate(program, position, n);
		case Op::Ternary: {
			long long condition = Evaluate(program, position, n);
			long long when_true = Evaluate(program, position, n);
			long long when_false = Evaluate(program, position, n);
			return condition ? when_true : when_false;
		}
		}
		return 0;
	}
};

struct Catalog {
	std::unordered_map<std::string, std::string> messages;
	std::unordered_map<std::string, std::vector<std::string>> plurals;
	PluralRule plural_rule;
};

Catalog catalog;
std::string active_language;

bool ReadCatalog(fsp::path const &path, Catalog &out) {
	std::ifstream file(path, std::ios::binary);
	if (!file) return false;
	std::string data((std::istreambuf_iterator<char>(file)), std::istreambuf_iterator<char>());
	if (data.size() < 28) return false;
	if (ReadU32(data, 0) != 0x950412de) return false;

	std::uint32_t count = ReadU32(data, 8);
	std::uint32_t originals = ReadU32(data, 12);
	std::uint32_t translations = ReadU32(data, 16);
	if (count > 1000000) return false;

	auto read_string = [&](std::uint32_t length, std::uint32_t offset) {
		if (offset > data.size() || length > data.size() - offset) return std::string();
		return data.substr(offset, length);
	};

	Catalog parsed;
	for (std::uint32_t i = 0; i < count; ++i) {
		std::uint32_t original_length = ReadU32(data, originals + i * 8);
		std::uint32_t original_offset = ReadU32(data, originals + i * 8 + 4);
		std::uint32_t translation_length = ReadU32(data, translations + i * 8);
		std::uint32_t translation_offset = ReadU32(data, translations + i * 8 + 4);
		std::string original = read_string(original_length, original_offset);
		std::string translation = read_string(translation_length, translation_offset);
		if (original.empty() && !translation.empty()) {
			parsed.plural_rule.Configure(translation);
			continue;
		}
		if (original.empty()) continue;

		auto nul = original.find('\0');
		if (nul != std::string::npos) {
			std::vector<std::string> forms;
			size_t start = 0;
			while (start <= translation.size()) {
				auto end = translation.find('\0', start);
				if (end == std::string::npos) {
					forms.push_back(translation.substr(start));
					break;
				}
				forms.push_back(translation.substr(start, end - start));
				start = end + 1;
			}
			parsed.plurals.emplace(std::move(original), std::move(forms));
		}
		else
			parsed.messages.emplace(std::move(original), std::move(translation));
	}

	out = std::move(parsed);
	return true;
}

std::vector<fsp::path> CatalogDirectories() {
	std::vector<fsp::path> directories;
	if (char const *override_dir = std::getenv("AEGISUB_LOCALE_DIR"))
		if (*override_dir) directories.emplace_back(override_dir);
	if (config::path)
		directories.push_back(config::path->Decode("?data/locale/"));
#ifdef P_LOCALE
	directories.emplace_back(P_LOCALE);
#endif
#if defined(__APPLE__)
	auto executable = agi::util::ExecutablePath();
	if (!executable.empty()) {
		// aegisub.app/Contents/MacOS/aegisub -> Contents/Resources
		auto resources = executable.parent_path().parent_path() / "Resources";
		directories.push_back(resources / "locale");
		directories.push_back(resources);
	}
#endif
	return directories;
}

fsp::path FindCatalog(std::string const &code) {
	if (code.empty()) return {};
	std::string file = CatalogName() + ".mo";
	for (auto const &directory : CatalogDirectories()) {
		std::error_code error;
		auto nested = directory / code / "LC_MESSAGES" / file;
		if (fsp::is_regular_file(nested, error)) return nested;
		auto flat = directory / (code + ".mo");
		if (fsp::is_regular_file(flat, error)) return flat;
	}
	return {};
}

std::string NormalizeLanguage(std::string value) {
	auto dot = value.find_first_of(".@");
	if (dot != std::string::npos) value.resize(dot);
	std::replace(value.begin(), value.end(), '-', '_');
	for (auto &c : value) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
	return value;
}

std::string SystemLanguage() {
#if defined(_WIN32)
	wchar_t buffer[LOCALE_NAME_MAX_LENGTH]{};
	if (GetLocaleInfoEx(LOCALE_NAME_USER_DEFAULT, LOCALE_SNAME, buffer, LOCALE_NAME_MAX_LENGTH) > 0) {
		std::wstring wide(buffer);
		std::string narrow;
		int size = WideCharToMultiByte(CP_UTF8, 0, wide.data(), static_cast<int>(wide.size()), nullptr, 0, nullptr, nullptr);
		narrow.resize(size);
		WideCharToMultiByte(CP_UTF8, 0, wide.data(), static_cast<int>(wide.size()), narrow.data(), size, nullptr, nullptr);
		return NormalizeLanguage(narrow);
	}
#elif defined(__APPLE__)
	CFLocaleRef locale = CFLocaleCopyCurrent();
	std::string identifier;
	if (locale) {
		CFStringRef name = static_cast<CFStringRef>(CFLocaleGetValue(locale, kCFLocaleIdentifier));
		char buffer[128]{};
		if (name && CFStringGetCString(name, buffer, sizeof buffer, kCFStringEncodingUTF8))
			identifier = buffer;
		CFRelease(locale);
	}
	if (!identifier.empty()) return NormalizeLanguage(identifier);
#else
	for (char const *variable : {"LC_ALL", "LC_MESSAGES", "LANG"}) {
		char const *value = std::getenv(variable);
		if (value && *value) return NormalizeLanguage(value);
	}
#endif
	return {};
}
}

namespace agi::i18n {
void SetLanguage(std::string const &code) {
	catalog = Catalog{};
	active_language.clear();

	std::string normalized = NormalizeLanguage(code);
	if (normalized.empty() || normalized == "en_us" || normalized == "c" || normalized == "posix") {
		std::setlocale(LC_ALL, "");
		std::setlocale(LC_NUMERIC, "C");
		return;
	}

	auto path = FindCatalog(code);
	if (path.empty()) path = FindCatalog(normalized);
	if (path.empty()) {
		std::fprintf(stderr, "[locale] %s catalog missing\n", code.c_str());
		return;
	}

	Catalog loaded;
	if (!ReadCatalog(path, loaded)) {
		std::fprintf(stderr, "[locale] %s catalog unreadable\n", path.string().c_str());
		return;
	}
	catalog = std::move(loaded);
	active_language = code;
	std::fprintf(stderr, "[locale] %s catalog loaded\n", code.c_str());

	std::setlocale(LC_ALL, "");
	std::setlocale(LC_NUMERIC, "C");
}

std::string const &ActiveLanguage() { return active_language; }

std::vector<std::string> AvailableLanguages() {
	std::vector<std::string> codes;
	std::string file = CatalogName() + ".mo";
	for (auto const &directory : CatalogDirectories()) {
		std::error_code error;
		for (auto const &entry : fsp::directory_iterator(directory, error)) {
			if (!entry.is_directory(error)) continue;
			auto candidate = entry.path() / "LC_MESSAGES" / file;
			if (!fsp::is_regular_file(candidate, error)) continue;
			codes.push_back(entry.path().filename().string());
		}
		if (error) continue;
		for (auto const &entry : fsp::directory_iterator(directory, error)) {
			if (!entry.is_regular_file(error) || entry.path().extension() != ".mo") continue;
			codes.push_back(entry.path().stem().string());
		}
	}
	std::sort(codes.begin(), codes.end());
	codes.erase(std::unique(codes.begin(), codes.end()), codes.end());
	codes.erase(std::remove(codes.begin(), codes.end(), "en_US"), codes.end());
	return codes;
}

bool HasLanguage(std::string const &code) {
	if (code == "en_US" || code.empty()) return true;
	auto codes = AvailableLanguages();
	return std::find(codes.begin(), codes.end(), code) != codes.end();
}

std::string RecommendLanguage() {
	auto codes = AvailableLanguages();
	std::string system = SystemLanguage();
	if (!system.empty()) {
		for (auto const &code : codes) {
			if (NormalizeLanguage(code) == system) return code;
		}
		auto base = system.substr(0, system.find('_'));
		for (auto const &code : codes) {
			if (NormalizeLanguage(code) == base) return code;
		}
		for (auto const &code : codes) {
			if (NormalizeLanguage(code).starts_with(base + "_")) return code;
		}
	}
	return "en_US";
}

std::string LanguageName(std::string const &code) {
	if (code == "en_US") return "English (United States)";
	icu::Locale locale(code.c_str());
	if (!locale.isBogus()) {
		icu::UnicodeString name;
		locale.getDisplayName(locale, name);
		if (!name.isEmpty()) {
			std::string utf8;
			name.toUTF8String(utf8);
			return utf8;
		}
	}
	return code;
}

std::string Translate(char const *msgid) {
	if (!msgid) return {};
	if (catalog.messages.empty()) return msgid;
	auto it = catalog.messages.find(msgid);
	return it == catalog.messages.end() ? msgid : it->second;
}

std::string TranslateContext(char const *context, char const *msgid) {
	if (!msgid) return {};
	if (!context || !*context) return Translate(msgid);
	std::string key = std::string(context) + '\x04' + msgid;
	auto it = catalog.messages.find(key);
	return it == catalog.messages.end() ? msgid : it->second;
}

std::string TranslatePlural(char const *singular, char const *plural, int n) {
	if (!singular) return {};
	std::string key = std::string(singular) + '\0' + plural;
	auto it = catalog.plurals.find(key);
	if (it != catalog.plurals.end() && !it->second.empty()) {
		int index = catalog.plural_rule.index(n);
		if (index >= static_cast<int>(it->second.size())) index = 0;
		return it->second[index];
	}
	it = catalog.plurals.find(singular);
	if (it == catalog.plurals.end()) {
		auto single = catalog.messages.find(singular);
		if (single != catalog.messages.end() && n == 1) return single->second;
	}
	return n == 1 ? singular : plural;
}
}
