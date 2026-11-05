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

#pragma once

#include <libaegisub/fs.h>

#include <memory>
#include <string>
#include <utility>
#include <vector>

/// The small XML subset the TTXT subtitle format needs: elements,
/// attributes, character data and the standard entities.
namespace xml {
struct Node {
	explicit Node(std::string name) : name(std::move(name)) { }

	std::string name;
	std::vector<std::pair<std::string, std::string>> attributes;
	std::vector<std::unique_ptr<Node>> children;
	std::string text;

	std::string const& Attribute(std::string const& key) const;
	std::string Attribute(std::string const& key, std::string const& fallback) const;
	void SetAttribute(std::string key, std::string value);
	Node& AddChild(std::unique_ptr<Node> child);
	Node& AddChild(std::string child_name);

	/// Text of this node and all of its descendants.
	std::string Content() const;
};

/// Parse a complete document and return its root element.
/// @throws std::runtime_error on malformed input
std::unique_ptr<Node> Parse(std::string const& document);

std::string Serialize(Node const& root);

std::string ReadFile(agi::fs::path const& filename);
void WriteFile(agi::fs::path const& filename, Node const& root);
}
