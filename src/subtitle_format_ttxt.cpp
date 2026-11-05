// Copyright (c) 2007, Rodrigo Braz Monteiro
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

/// @file subtitle_format_ttxt.cpp
/// @brief Reading/writing MPEG-4 Timed Text subtitles in TTXT XML format
/// @ingroup subtitle_io
///

#include "subtitle_format_ttxt.h"

#include "ass_dialogue.h"
#include "ass_file.h"
#include "options.h"

#include <memory>

#include <libaegisub/ass/time.h>

#include "xml_dom.h"

DEFINE_EXCEPTION(TTXTParseError, SubtitleFormatParseError);

TTXTSubtitleFormat::TTXTSubtitleFormat()
: SubtitleFormat("MPEG-4 Streaming Text")
{
}

std::vector<std::string> TTXTSubtitleFormat::GetReadWildcards() const {
	return {"ttxt"};
}

std::vector<std::string> TTXTSubtitleFormat::GetWriteWildcards() const {
	return GetReadWildcards();
}

void TTXTSubtitleFormat::ReadFile(AssFile *target, agi::fs::path const& filename, agi::vfr::Framerate const&, const char *) const {
	target->LoadDefault(false, OPT_GET("Subtitle Format/TTXT/Default Style Catalog")->GetString());

	// Load XML document
	std::unique_ptr<xml::Node> document;
	try {
		document = xml::Parse(xml::ReadFile(filename));
	}
	catch (std::exception const&) {
		throw TTXTParseError("Failed loading TTXT XML file.");
	}

	// Check root node name
	if (document->name != "TextStream") throw TTXTParseError("Invalid TTXT file.");

	// Check version
	std::string verStr = document->Attribute("version");
	int version = -1;
	if (verStr == "1.0")
		version = 0;
	else if (verStr == "1.1")
		version = 1;
	else
		throw TTXTParseError("Unknown TTXT version: " + verStr);

	// Get children
	AssDialogue *diag = nullptr;
	int lines = 0;
	for (auto const& child : document->children) {
		// Line
		if (child->name == "TextSample") {
			if ((diag = ProcessLine(child.get(), diag, version))) {
				lines++;
				target->Events.push_back(*diag);
			}
		}
		// Header
		else if (child->name == "TextStreamHeader") {
			ProcessHeader(child.get());
		}
	}

	// No lines?
	if (lines == 0)
		target->Events.push_back(*new AssDialogue);
}

AssDialogue *TTXTSubtitleFormat::ProcessLine(xml::Node *node, AssDialogue *prev, int version) const {
	// Get time
	std::string sampleTime = node->Attribute("sampleTime", "00:00:00.000");
	agi::Time time(sampleTime);

	// Set end time of last line
	if (prev)
		prev->End = time;

	// Get text
	std::string text;
	if (version == 0)
		text = node->Attribute("text");
	else
		text = node->Content();

	// Create line
	if (text.empty()) return nullptr;

	// Create dialogue
	auto diag = new AssDialogue;
	diag->Start = time;
	diag->End = 36000000-10;

	// Process text for 1.0
	if (version == 0) {
		std::string finalText;
		finalText.reserve(text.size());
		bool in = false;
		bool first = true;
		for (auto chr : text) {
			if (chr == '\'') {
				if (!in && !first) finalText += "\\N";
				first = false;
				in = !in;
			}
			else if (in) finalText += chr;
		}
		diag->Text = finalText;
	}

	// Process text for 1.1
	else {
		std::string collapsed;
		collapsed.reserve(text.size());
		for (char c : text) {
			if (c == '\r') continue;
			if (c == '\n') collapsed += "\\N";
			else collapsed += c;
		}
		diag->Text = std::move(collapsed);
	}

	return diag;
}

void TTXTSubtitleFormat::ProcessHeader(xml::Node *) const {
	// TODO
}

void TTXTSubtitleFormat::WriteFile(const AssFile *src, agi::fs::path const& filename, agi::vfr::Framerate const& , const char *) const {
	// Convert to TTXT
	AssFile copy(*src);
	ConvertToTTXT(copy);

	xml::Node root("TextStream");
	root.SetAttribute("version", "1.1");

	// Create header
	WriteHeader(&root);

	// Create lines
	const AssDialogue *prev = nullptr;
	for (auto const& current : copy.Events) {
		WriteLine(&root, prev, &current);
		prev = &current;
	}

	// Save XML
	xml::WriteFile(filename, root);
}

void TTXTSubtitleFormat::WriteHeader(xml::Node *root) const {
	// Write stream header
	auto& header = root->AddChild("TextStreamHeader");
	header.SetAttribute("width", "400");
	header.SetAttribute("height", "60");
	header.SetAttribute("layer", "0");
	header.SetAttribute("translation_x", "0");
	header.SetAttribute("translation_y", "0");

	// Write sample description
	auto& description = header.AddChild("TextSampleDescription");
	description.SetAttribute("horizontalJustification", "center");
	description.SetAttribute("verticalJustification", "bottom");
	description.SetAttribute("backColor", "0 0 0 0");
	description.SetAttribute("verticalText", "no");
	description.SetAttribute("fillTextRegion", "no");
	description.SetAttribute("continuousKaraoke", "no");
	description.SetAttribute("scroll", "None");

	// Write font table
	auto& font_table = description.AddChild("FontTable");
	auto& font_entry = font_table.AddChild("FontTableEntry");
	font_entry.SetAttribute("fontName", "Sans");
	font_entry.SetAttribute("fontID", "1");

	// Write text box
	auto& text_box = description.AddChild("TextBox");
	text_box.SetAttribute("top", "0");
	text_box.SetAttribute("left", "0");
	text_box.SetAttribute("bottom", "60");
	text_box.SetAttribute("right", "400");

	// Write style
	auto& style = description.AddChild("Style");
	style.SetAttribute("styles", "Normal");
	style.SetAttribute("fontID", "1");
	style.SetAttribute("fontSize", "18");
	style.SetAttribute("color", "ff ff ff ff");
}

void TTXTSubtitleFormat::WriteLine(xml::Node *root, const AssDialogue *prev, const AssDialogue *line) const {
	// If it doesn't start at the end of previous, add blank
	if (prev && prev->End != line->Start) {
		auto& node = root->AddChild("TextSample");
		node.SetAttribute("sampleTime", "0" + prev->End.GetAssFormatted(true));
		node.SetAttribute("xml:space", "preserve");
	}

	// Generate and insert node
	auto& node = root->AddChild("TextSample");
	node.SetAttribute("sampleTime", "0" + line->Start.GetAssFormatted(true));
	node.SetAttribute("xml:space", "preserve");
	node.text = line->Text;
}

void TTXTSubtitleFormat::ConvertToTTXT(AssFile &file) const {
	file.Sort();
	StripComments(file);
	RecombineOverlaps(file);
	MergeIdentical(file);
	StripTags(file);
	ConvertNewlines(file, "\r\n");

	// Find last line
	agi::Time lastTime;
	if (!file.Events.empty())
		lastTime = file.Events.back().End;

	// Insert blank line at the end
	auto diag = new AssDialogue;
	diag->Start = lastTime;
	diag->End = lastTime+OPT_GET("Timing/Default Duration")->GetInt();
	file.Events.push_back(*diag);
}
