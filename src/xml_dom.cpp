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

#include "xml_dom.h"

#include <libaegisub/fs.h>

#include <cctype>
#include <fstream>
#include <sstream>
#include <stdexcept>

namespace {
void append_utf8(std::string &out, unsigned long code) {
	if (code < 0x80) {
		out += static_cast<char>(code);
	}
	else if (code < 0x800) {
		out += static_cast<char>(0xC0 | (code >> 6));
		out += static_cast<char>(0x80 | (code & 0x3F));
	}
	else if (code < 0x10000) {
		out += static_cast<char>(0xE0 | (code >> 12));
		out += static_cast<char>(0x80 | ((code >> 6) & 0x3F));
		out += static_cast<char>(0x80 | (code & 0x3F));
	}
	else {
		out += static_cast<char>(0xF0 | (code >> 18));
		out += static_cast<char>(0x80 | ((code >> 12) & 0x3F));
		out += static_cast<char>(0x80 | ((code >> 6) & 0x3F));
		out += static_cast<char>(0x80 | (code & 0x3F));
	}
}

std::string decode_entities(std::string_view input) {
	std::string out;
	out.reserve(input.size());
	for (size_t i = 0; i < input.size(); ++i) {
		if (input[i] != '&') {
			out += input[i];
			continue;
		}
		auto end = input.find(';', i);
		if (end == std::string_view::npos) {
			out += input[i];
			continue;
		}
		std::string_view entity = input.substr(i + 1, end - i - 1);
		if (entity == "amp") out += '&';
		else if (entity == "lt") out += '<';
		else if (entity == "gt") out += '>';
		else if (entity == "quot") out += '"';
		else if (entity == "apos") out += '\'';
		else if (entity.size() > 1 && entity[0] == '#') {
			bool hex = entity[1] == 'x' || entity[1] == 'X';
			std::string digits(entity.substr(hex ? 2 : 1));
			char *stop = nullptr;
			unsigned long code = std::strtoul(digits.c_str(), &stop, hex ? 16 : 10);
			if (stop && *stop == '\0')
				append_utf8(out, code);
		}
		else {
			out.append(input.substr(i, end - i + 1));
		}
		i = end;
	}
	return out;
}

std::string escape(std::string const& input, bool attribute) {
	std::string out;
	out.reserve(input.size());
	for (char c : input) {
		switch (c) {
			case '&': out += "&amp;"; break;
			case '<': out += "&lt;"; break;
			case '>': out += "&gt;"; break;
			case '"':
				if (attribute) out += "&quot;";
				else out += c;
				break;
			default: out += c; break;
		}
	}
	return out;
}

class Parser {
	std::string_view input;
	size_t pos = 0;

	[[noreturn]] void fail(std::string const& message) const {
		throw std::runtime_error("XML parse error at byte " + std::to_string(pos) + ": " + message);
	}

	bool starts_with(std::string_view value) const {
		return input.substr(pos, value.size()) == value;
	}

	void skip_space() {
		while (pos < input.size() && std::isspace(static_cast<unsigned char>(input[pos])))
			++pos;
	}

	void skip_misc() {
		for (;;) {
			skip_space();
			if (starts_with("<?")) {
				auto end = input.find("?>", pos);
				if (end == std::string_view::npos) fail("unterminated processing instruction");
				pos = end + 2;
			}
			else if (starts_with("<!--")) {
				auto end = input.find("-->", pos);
				if (end == std::string_view::npos) fail("unterminated comment");
				pos = end + 3;
			}
			else if (starts_with("<!DOCTYPE")) {
				auto end = input.find('>', pos);
				if (end == std::string_view::npos) fail("unterminated doctype");
				pos = end + 1;
			}
			else {
				return;
			}
		}
	}

	std::string name() {
		size_t start = pos;
		while (pos < input.size()) {
			char c = input[pos];
			if (std::isspace(static_cast<unsigned char>(c)) || c == '>' || c == '/' || c == '=')
				break;
			++pos;
		}
		if (start == pos) fail("expected a name");
		return std::string(input.substr(start, pos - start));
	}

	std::string quoted_value() {
		if (pos >= input.size() || (input[pos] != '"' && input[pos] != '\''))
			fail("expected a quoted attribute value");
		char quote = input[pos++];
		size_t start = pos;
		while (pos < input.size() && input[pos] != quote)
			++pos;
		if (pos >= input.size()) fail("unterminated attribute value");
		std::string value(decode_entities(input.substr(start, pos - start)));
		++pos;
		return value;
	}

	std::unique_ptr<xml::Node> element() {
		if (pos >= input.size() || input[pos] != '<') fail("expected an element");
		++pos;
		auto node = std::make_unique<xml::Node>(name());
		for (;;) {
			skip_space();
			if (starts_with("/>")) {
				pos += 2;
				return node;
			}
			if (pos < input.size() && input[pos] == '>') {
				++pos;
				break;
			}
			std::string key = name();
			skip_space();
			if (pos >= input.size() || input[pos] != '=') fail("expected '=' after an attribute name");
			++pos;
			skip_space();
			node->SetAttribute(std::move(key), quoted_value());
		}

		for (;;) {
			if (pos >= input.size()) fail("unterminated element " + node->name);
			if (starts_with("</")) {
				pos += 2;
				std::string closing = name();
				if (closing != node->name) fail("mismatched closing tag " + closing);
				skip_space();
				if (pos >= input.size() || input[pos] != '>') fail("expected '>'");
				++pos;
				return node;
			}
			if (starts_with("<!--")) {
				auto end = input.find("-->", pos);
				if (end == std::string_view::npos) fail("unterminated comment");
				pos = end + 3;
				continue;
			}
			if (starts_with("<![CDATA[")) {
				auto end = input.find("]]>", pos);
				if (end == std::string_view::npos) fail("unterminated CDATA");
				node->text.append(input.substr(pos + 9, end - pos - 9));
				pos = end + 3;
				continue;
			}
			if (starts_with("<?")) {
				auto end = input.find("?>", pos);
				if (end == std::string_view::npos) fail("unterminated processing instruction");
				pos = end + 2;
				continue;
			}
			if (input[pos] == '<') {
				node->children.push_back(element());
				continue;
			}
			size_t start = pos;
			while (pos < input.size() && input[pos] != '<')
				++pos;
			node->text.append(decode_entities(input.substr(start, pos - start)));
		}
	}

public:
	explicit Parser(std::string_view document) : input(document) { }

	std::unique_ptr<xml::Node> document() {
		if (starts_with("\xEF\xBB\xBF")) pos = 3;
		skip_misc();
		auto root = element();
		skip_misc();
		return root;
	}
};

void serialize_node(std::ostringstream &out, xml::Node const& node, int depth) {
	out << std::string(depth, '\t') << '<' << node.name;
	for (auto const& [key, value] : node.attributes)
		out << ' ' << key << "=\"" << escape(value, true) << '"';
	if (node.children.empty() && node.text.empty()) {
		out << "/>\n";
		return;
	}
	out << '>';
	if (node.children.empty()) {
		out << escape(node.text, false) << "</" << node.name << ">\n";
		return;
	}
	out << '\n';
	for (auto const& child : node.children)
		serialize_node(out, *child, depth + 1);
	if (!node.text.empty())
		out << std::string(depth + 1, '\t') << escape(node.text, false) << '\n';
	out << std::string(depth, '\t') << "</" << node.name << ">\n";
}
}

namespace xml {
std::string const& Node::Attribute(std::string const& key) const {
	static std::string const empty;
	for (auto const& [name, value] : attributes)
		if (name == key) return value;
	return empty;
}

std::string Node::Attribute(std::string const& key, std::string const& fallback) const {
	for (auto const& [name, value] : attributes)
		if (name == key) return value;
	return fallback;
}

void Node::SetAttribute(std::string key, std::string value) {
	for (auto& [name, existing] : attributes) {
		if (name == key) {
			existing = std::move(value);
			return;
		}
	}
	attributes.emplace_back(std::move(key), std::move(value));
}

Node& Node::AddChild(std::unique_ptr<Node> child) {
	children.push_back(std::move(child));
	return *children.back();
}

Node& Node::AddChild(std::string child_name) {
	return AddChild(std::make_unique<Node>(std::move(child_name)));
}

std::string Node::Content() const {
	std::string out = text;
	for (auto const& child : children)
		out += child->Content();
	return out;
}

std::unique_ptr<Node> Parse(std::string const& document) {
	return Parser(document).document();
}

std::string Serialize(Node const& root) {
	std::ostringstream out;
	out << "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n";
	serialize_node(out, root, 0);
	return out.str();
}

std::string ReadFile(agi::fs::path const& filename) {
	std::ifstream file(filename, std::ios::binary);
	if (!file) throw std::runtime_error("cannot open XML file");
	std::ostringstream buffer;
	buffer << file.rdbuf();
	return buffer.str();
}

void WriteFile(agi::fs::path const& filename, Node const& root) {
	std::ofstream file(filename, std::ios::binary | std::ios::trunc);
	if (!file) throw std::runtime_error("cannot open XML file for writing");
	file << Serialize(root);
	if (!file) throw std::runtime_error("cannot write XML file");
}
}
