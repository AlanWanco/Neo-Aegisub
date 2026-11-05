// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

#include "gpui_document.h"
#include "localisation.h"


#include "ass_file.h"
#include "autosave.h"
#include "ass_attachment.h"
#include "ass_dialogue.h"
#include "ass_info.h"
#include "ass_style.h"
#include "ass_style_storage.h"
#include "charset_detect.h"
#include "options.h"
#include "subtitle_format.h"
#include "async_video_provider.h"
#include "include/aegisub/context.h"
#include "project.h"
#include "resolution_resampler.h"
#include "style_renamer.h"
#include "selection_controller.h"
#include "video_controller.h"
#include "image_png.h"

namespace {
std::string base64(unsigned char const *data, size_t size) {
	static char const alphabet[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
	std::string out;
	out.reserve((size + 2) / 3 * 4);
	for (size_t i = 0; i < size; i += 3) {
		unsigned value = static_cast<unsigned>(data[i]) << 16;
		if (i + 1 < size) value |= static_cast<unsigned>(data[i + 1]) << 8;
		if (i + 2 < size) value |= data[i + 2];
		out += alphabet[(value >> 18) & 63];
		out += alphabet[(value >> 12) & 63];
		out += i + 1 < size ? alphabet[(value >> 6) & 63] : '=';
		out += i + 2 < size ? alphabet[value & 63] : '=';
	}
	return out;
}
}

#include "video_frame.h"
#include "video_provider_dummy.h"
#include "include/aegisub/subtitles_provider.h"


#include <libaegisub/cajun/writer.h>
#include <libaegisub/audio/provider.h>
#include <libaegisub/charset.h>
#include <libaegisub/charset_conv.h>
#include <libaegisub/keyframe.h>
#include <libaegisub/path.h>

#include <boost/algorithm/string/predicate.hpp>

#include <algorithm>
#include <charconv>
#include <cctype>
#include <cmath>
#include <iomanip>
#include <limits>
#include <map>
#include <set>
#include <sstream>
#include <stdexcept>

namespace {
const std::set<std::string, std::less<>> script_fields = {
	"Title", "Original Script", "Original Translation", "Original Editing", "Original Timing",
	"Synch Point", "Script Updated By", "Update Details", "PlayResX", "PlayResY", "LayoutResX", "LayoutResY",
	"WrapStyle", "ScaledBorderAndShadow", "YCbCr Matrix"
};

const std::map<std::string, double AssStyle::*, std::less<>> decimal_fields = {
	{"fontsize", &AssStyle::fontsize}, {"scalex", &AssStyle::scalex}, {"scaley", &AssStyle::scaley},
	{"spacing", &AssStyle::spacing}, {"angle", &AssStyle::angle},
	{"outline_w", &AssStyle::outline_w}, {"shadow_w", &AssStyle::shadow_w}
};
const std::map<std::string, bool AssStyle::*, std::less<>> boolean_fields = {
	{"bold", &AssStyle::bold}, {"italic", &AssStyle::italic},
	{"underline", &AssStyle::underline}, {"strikeout", &AssStyle::strikeout}
};
const std::map<std::string, agi::Color AssStyle::*, std::less<>> color_fields = {
	{"primary", &AssStyle::primary}, {"secondary", &AssStyle::secondary},
	{"outline", &AssStyle::outline}, {"shadow", &AssStyle::shadow}
};

std::string encode(json::Object object) {
	std::ostringstream stream;
	agi::JsonWriter::Write(object, stream);
	auto response = stream.str();
	response.erase(std::remove(response.begin(), response.end(), '\n'), response.end());
	return response;
}

int parse_integer(std::string const& text, int lower, int upper) {
	int result;
	auto [end, error] = std::from_chars(text.data(), text.data() + text.size(), result);
	if (error != std::errc{} || end != text.data() + text.size() || result < lower || result > upper)
		throw std::invalid_argument("integer is outside the permitted range");
	return result;
}

bool parse_boolean(std::string const& value) {
	if (value != "0" && value != "1") throw std::invalid_argument("enter 0 or 1 for boolean fields");
	return value == "1";
}

void check_single_line(std::string const& value, bool no_comma = false) {
	if (value.find_first_of("\r\n") != value.npos || (no_comma && value.find(',') != value.npos))
		throw std::invalid_argument("ASS fields cannot contain newlines or unescaped separators");
}

void set_style_field(AssStyle& style, std::string const& key, std::string const& value) {
	check_single_line(value);
	if (key == "name" || key == "font") {
		check_single_line(value, true);
		if (value.empty() || value.find_first_not_of(" \t") == value.npos)
			throw std::invalid_argument("style name and font cannot be empty");
		(key == "name" ? style.name : style.font) = value;
	}
	else if (auto field = decimal_fields.find(key); field != decimal_fields.end()) {
		size_t consumed;
		double const number = std::stod(value, &consumed);
		if (consumed != value.size() || !std::isfinite(number) || std::abs(number) > 1000000 ||
			((key == "fontsize" || key == "scalex" || key == "scaley" || key == "outline_w") && number < 0))
			throw std::invalid_argument("style number is outside the permitted range");
		style.*field->second = number;
	}
	else if (auto field = boolean_fields.find(key); field != boolean_fields.end())
		style.*field->second = parse_boolean(value);
	else if (auto field = color_fields.find(key); field != color_fields.end()) {
		// Hex RGBA is unambiguous and preserves ASS transparency (00 is opaque).
		if ((value.size() != 7 && value.size() != 9) || value.front() != '#')
			throw std::invalid_argument("enter colors as #RRGGBB or #RRGGBBAA (ASS transparency)");
		for (size_t i = 1; i < value.size(); ++i)
			if (!std::isxdigit(static_cast<unsigned char>(value[i])))
				throw std::invalid_argument("invalid hexadecimal color");
		auto component = [&](size_t offset) {
			unsigned int result = 0;
			std::from_chars(value.data() + offset, value.data() + offset + 2, result, 16);
			return static_cast<unsigned char>(result);
		};
		style.*field->second = agi::Color(component(1), component(3), component(5), value.size() == 9 ? component(7) : 0);
	}
	else if (key == "alignment") style.alignment = parse_integer(value, 1, 9);
	else if (key == "borderstyle") {
		int const number = parse_integer(value, 1, 4);
		if (number == 2) throw std::invalid_argument("borderstyle must be 1, 3 or 4");
		style.borderstyle = number;
	}
	else if (key == "encoding") style.encoding = parse_integer(value, 0, 255);
	else if (key == "margin_left") style.Margin[0] = parse_integer(value, -9999, 99999);
	else if (key == "margin_right") style.Margin[1] = parse_integer(value, -9999, 99999);
	else if (key == "margin_vertical") style.Margin[2] = parse_integer(value, -9999, 99999);
	else throw std::invalid_argument("unknown style field");
}

/// Renders the style being edited through the real subtitles provider so the
/// preview matches what the video will show. Drawing it in the frontend cannot
/// reproduce libass: the font lookup, the outline stacking and the shadow
/// offsets are all provider behaviour.
std::string style_preview(agi::Context *context, json::Object const& request) {
	AssStyle style;
	auto const& fields = static_cast<json::Object const&>(request.at("fields"));
	for (auto const& [key, value] : fields) set_style_field(style, key, static_cast<std::string const&>(value));
	style.name = "Default";
	// Centred with no margins: the preview shows the style, not the layout.
	style.alignment = 5;
	std::fill(style.Margin.begin(), style.Margin.end(), 0);
	style.UpdateData();

	int const width = parse_integer(static_cast<std::string const&>(request.at("width")), 16, 4096);
	int const height = parse_integer(static_cast<std::string const&>(request.at("height")), 16, 1024);
	// Same preview text the wx style editor uses, so both show one string.
	std::string const text = OPT_GET("Tool/Style Editor/Preview Text")->GetString();

	AssFile file;
	file.Styles.clear();
	file.Events.clear();
	file.Styles.push_back(*new AssStyle(style));
	auto line = new AssDialogue;
	line->Style = style.name;
	line->Text = "{\\q2}" + text;
	file.Events.push_back(*line);
	file.SetScriptInfo("PlayResX", std::to_string(width));
	file.SetScriptInfo("PlayResY", std::to_string(height));

	// The provider is expensive to create and holds the loaded font face, so it
	// outlives a single preview.
	static std::unique_ptr<SubtitlesProvider> provider;
	if (!provider) provider = SubtitlesProviderFactory::GetProvider(nullptr);
	provider->LoadSubtitles(&file);

	// The preview keeps the style editor's own background colour rather than the
	// workspace theme: light text has to stay readable on a light theme.
	agi::Color background = OPT_GET("Colour/Style Editor/Background/Preview")->GetColor();
	DummyVideoProvider video(agi::vfr::Framerate(), 10, width, height, background, false);
	VideoFrame frame;
	video.GetFrame(0, frame);
	provider->DrawSubtitles(frame, 0.1);

	RasterImage image = GetRgbImage(frame);
	auto bytes = EncodePng(image.width, image.height, image.data.data(), false);

	json::Object result;
	result["ok"] = true;
	result["width"] = std::to_string(width);
	result["height"] = std::to_string(height);
	result["image"] = base64(bytes.data(), bytes.size());
	return encode(std::move(result));
}

json::Object describe_style(AssStyle const& style) {
	json::Object fields;
	fields["name"] = style.name;
	fields["font"] = style.font;
	for (auto const& [name, member] : decimal_fields) {
		std::ostringstream value;
		value << std::setprecision(15) << style.*member;
		fields[name] = value.str();
	}
	for (auto const& [name, member] : boolean_fields) fields[name] = style.*member ? "1" : "0";
	for (auto const& [name, member] : color_fields) fields[name] = (style.*member).GetHexFormatted(true);
	fields["borderstyle"] = std::to_string(style.borderstyle);
	fields["alignment"] = std::to_string(style.alignment);
	fields["encoding"] = std::to_string(style.encoding);
	fields["margin_left"] = std::to_string(style.Margin[0]);
	fields["margin_right"] = std::to_string(style.Margin[1]);
	fields["margin_vertical"] = std::to_string(style.Margin[2]);
	return fields;
}
}

namespace gpui {
std::string FileTools(json::Object const& request) {
	std::string const& action = request.at("action");
	if (action != "state" && action != "inspect") throw std::invalid_argument("unknown file tools action");
	json::Object result;
	result["ok"] = true;
	json::Array encodings;
	for (auto const& name : agi::charset::GetEncodingsList<std::vector<std::string>>()) encodings.emplace_back(name);
	result["encodings"] = std::move(encodings);
	if (action == "inspect") {
		std::string const& path = request.at("path");
		result["detected"] = agi::charset::Detect(agi::fs::path(path));
		result["ambiguous_encoding"] = static_cast<std::string const&>(result.at("detected")).empty();
		result["path"] = path;
	}
	json::Array files;
	for (auto const& file : GetAutosaveFiles()) {
		json::Object item;
		item["name"] = file.name;
		json::Array versions;
		for (auto const& version : file.versions) {
			json::Object v;
			v["path"] = version.filename;
			v["display"] = version.display;
			v["kind"] = version.kind;
			v["timestamp"] = version.timestamp;
			versions.emplace_back(std::move(v));
		}
		item["versions"] = std::move(versions);
		files.emplace_back(std::move(item));
	}
	result["files"] = std::move(files);
	return encode(std::move(result));
}

std::string SaveMedia(agi::Context *context, json::Object const& request) {
	std::string const& kind = request.at("kind");
	std::string const& filename = request.at("path");
	if (filename.empty()) throw std::invalid_argument("choose a destination file");
	agi::fs::path path(filename);
	bool overwrite = false;
	if (auto item = request.find("overwrite"); item != request.end()) overwrite = static_cast<json::Boolean const&>(item->second);
	if (agi::fs::Exists(path) && (!agi::fs::FileExists(path) || !overwrite)) throw std::invalid_argument("destination exists; confirm replacing the file");
	json::Object result;
	if (kind == "audio_clip") {
		auto provider = context->project->AudioProvider();
		auto const& selected = context->selectionController->GetSelectedSet();
		if (!provider || selected.empty()) throw std::invalid_argument("load audio and select subtitle lines before saving a clip");
		int start = std::numeric_limits<int>::max(), end = 0;
		for (auto line : selected) {
			if (line->End < line->Start) throw std::invalid_argument("selected line has a negative duration");
			start = std::min(start, static_cast<int>(line->Start));
			end = std::max(end, static_cast<int>(line->End));
		}
		agi::SaveAudioClip(*provider, path, start, end);
		result["start"] = start; result["end"] = end;
	}
	else if (kind == "keyframes") {
		if (context->project->Keyframes().empty()) throw std::invalid_argument("no keyframes are loaded");
		agi::keyframe::Save(path, context->project->Keyframes());
		config::mru->Add("Keyframes", path);
		OPT_SET("Path/Last/Keyframes")->SetString(path.parent_path().string());
		result["count"] = static_cast<int64_t>(context->project->Keyframes().size());
	}
	else if (kind == "timecodes") {
		if (!context->project->Timecodes().IsLoaded()) throw std::invalid_argument("no timecodes are loaded");
		auto provider = context->project->VideoProvider();
		context->project->Timecodes().Save(path, provider ? provider->GetFrameCount() : -1);
		config::mru->Add("Timecodes", path);
		OPT_SET("Path/Last/Timecodes")->SetString(path.parent_path().string());
	}
	else throw std::invalid_argument("unknown media save operation");
	result["ok"] = true;
	result["path"] = path.string();
	return encode(std::move(result));
}

std::string DocumentDetails(agi::Context *context) {
	json::Object result;
	result["ok"] = true;
	json::Array styles;
	for (auto const& style : context->ass->Styles) styles.emplace_back(describe_style(style));
	result["styles"] = std::move(styles);
	json::Object info;
	for (auto const& key : script_fields) info[key] = context->ass->GetScriptInfo(key);
	result["info"] = std::move(info);
	json::Object media;
	media["video"] = context->project->VideoName().string();
	media["audio"] = context->project->AudioName().string();
	media["keyframes"] = static_cast<int64_t>(context->project->Keyframes().size());
	media["timecodes"] = context->project->Timecodes().IsLoaded();
	if (auto video = context->project->VideoProvider()) {
		auto [width, height] = video->GetDisplayResolution();
		media["width"] = width;
		media["height"] = height;
		media["matrix"] = agi::ycbcr::Header(video->GetRealColorSpace()).to_best_practice_string();
	}
	result["media"] = std::move(media);
	return encode(std::move(result));
}

std::string Attachments(agi::Context *context, json::Object const& request, uint64_t const& revision) {
	std::string const& action = request.at("action");
	auto& attachments = context->ass->Attachments;
	if (action == "add") {
		std::string const& group = request.at("group");
		if (group != "font" && group != "graphic") throw std::invalid_argument("unknown attachment group");
		std::vector<AssAttachment> added;
		for (auto const& item : static_cast<json::Array const&>(request.at("paths"))) {
			agi::fs::path path(static_cast<std::string const&>(item));
			if (!IsSafeAttachmentFilename(path.filename().string())) throw std::invalid_argument("attachment has an invalid filename");
			added.emplace_back(path, group == "font" ? AssEntryGroup::FONT : AssEntryGroup::GRAPHIC);
		}
		if (!added.empty()) {
			attachments.insert(attachments.end(), added.begin(), added.end());
			context->ass->Commit(_("attach file"), AssFile::COMMIT_ATTACHMENT);
		}
	}
	else if (action == "delete" || action == "extract") {
		std::vector<size_t> indexes;
		for (auto const& item : static_cast<json::Array const&>(request.at("items"))) {
			auto const& entry = static_cast<json::Object const&>(item);
			int64_t const index = static_cast<json::Integer const&>(entry.at("index"));
			if (index < 0 || static_cast<size_t>(index) >= attachments.size()) throw std::invalid_argument("attachment no longer exists");
			std::string const& name = entry.at("name");
			if (attachments[index].GetFileName(true) != name) throw std::invalid_argument("attachment changed; refresh before continuing");
			indexes.push_back(static_cast<size_t>(index));
		}
		std::sort(indexes.begin(), indexes.end());
		indexes.erase(std::unique(indexes.begin(), indexes.end()), indexes.end());
		if (action == "delete") {
			for (auto index = indexes.rbegin(); index != indexes.rend(); ++index) attachments.erase(attachments.begin() + *index);
			if (!indexes.empty()) context->ass->Commit(_("remove attachment"), AssFile::COMMIT_ATTACHMENT);
		}
		else {
			std::string const& destination = request.at("path");
			agi::fs::path target(destination);
			bool const directory = static_cast<json::Boolean const&>(request.at("directory"));
			bool overwrite = false;
			if (auto value = request.find("overwrite"); value != request.end()) overwrite = static_cast<json::Boolean const&>(value->second);
			if (indexes.size() != 1 && !directory) throw std::invalid_argument("choose a directory when extracting multiple attachments");
			if (directory && !agi::fs::DirectoryExists(target)) throw std::invalid_argument("choose an existing output directory");
			std::vector<std::pair<size_t, agi::fs::path>> outputs;
			std::set<agi::fs::path> destinations;
			for (auto index : indexes) {
				auto name = attachments[index].GetFileName();
				if (directory && !IsSafeAttachmentFilename(name)) throw std::invalid_argument("embedded filename cannot be extracted into a directory; choose a destination file instead");
				auto path = directory ? target / name : target;
				if (!destinations.insert(path).second) throw std::invalid_argument("selected attachments share an output filename; extract them individually");
				if (agi::fs::Exists(path) && !overwrite) throw std::invalid_argument("output file already exists; confirm replacement before extracting");
				outputs.emplace_back(index, std::move(path));
			}
			for (auto const& [index, path] : outputs) attachments[index].Extract(path);
		}
	}
	else if (action != "state") throw std::invalid_argument("unknown attachment action");
	json::Array items;
	for (size_t i = 0; i < attachments.size(); ++i) {
		auto const& attachment = attachments[i];
		json::Object item;
		item["index"] = static_cast<int64_t>(i);
		item["name"] = attachment.GetFileName(true);
		item["filename"] = attachment.GetFileName();
		item["encoded_size"] = static_cast<int64_t>(attachment.GetSize());
		item["group"] = attachment.Group() == AssEntryGroup::FONT ? "font" : "graphic";
		item["safe_name"] = IsSafeAttachmentFilename(attachment.GetFileName());
		items.emplace_back(std::move(item));
	}
	json::Object result;
	result["ok"] = true;
	result["items"] = std::move(items);
	result["revision"] = static_cast<int64_t>(revision);
	return encode(std::move(result));
}

std::string Resample(agi::Context *context, json::Object const& request) {
	std::string const& action = request.at("action");
	if (action == "apply") {
		auto integer = [&](char const* name, int lower, int upper) {
			int64_t const value = static_cast<json::Integer const&>(request.at(name));
			if (value < lower || value > upper) throw std::invalid_argument(std::string(name) + " is outside the permitted range");
			return static_cast<int>(value);
		};
		ResampleSettings settings;
		settings.source_x = integer("source_x", 1, 999999);
		settings.source_y = integer("source_y", 1, 999999);
		settings.dest_x = integer("dest_x", 1, 999999);
		settings.dest_y = integer("dest_y", 1, 999999);
		int actual_x, actual_y;
		context->ass->GetResolution(actual_x, actual_y);
		if (auto expected = request.find("expected_source"); expected != request.end()) {
			auto const& size = static_cast<json::Array const&>(expected->second);
			if (size.size() != 2 || static_cast<json::Integer const&>(size[0]) != actual_x || static_cast<json::Integer const&>(size[1]) != actual_y)
				throw std::invalid_argument("script resolution changed; reload resampling settings");
		}
		settings.ar_mode = static_cast<ResampleARMode>(integer("ar_mode", 0, 3));
		char const* margins[] = {"left", "right", "top", "bottom"};
		if (settings.ar_mode == ResampleARMode::Manual) {
			for (size_t i = 0; i < 4; ++i) settings.margin[i] = integer(margins[i], -9999, 9999);
			if (settings.source_x + settings.margin[0] + settings.margin[1] <= 0 || settings.source_y + settings.margin[2] + settings.margin[3] <= 0)
				throw std::invalid_argument("manual borders must leave a positive source size");
		}
		std::string const& source_matrix = request.at("source_matrix");
		std::string const& dest_matrix = request.at("dest_matrix");
		if (!source_matrix.empty() || !dest_matrix.empty()) {
			agi::ycbcr::Header source(source_matrix), dest(dest_matrix);
			if (!std::holds_alternative<agi::ycbcr::header_colorspace>(source) || !std::holds_alternative<agi::ycbcr::header_colorspace>(dest))
				throw std::invalid_argument("choose explicit source and destination matrices, or disable color conversion");
			settings.matrix_conversion = std::make_pair(std::get<agi::ycbcr::header_colorspace>(source), std::get<agi::ycbcr::header_colorspace>(dest));
		}
		ResampleResolution(context->ass.get(), settings);
	}
	else if (action != "state") throw std::invalid_argument("unknown resampling action");
	json::Object result;
	result["ok"] = true;
	int width, height;
	context->ass->GetResolution(width, height);
	result["source_x"] = width;
	result["source_y"] = height;
	result["source_matrix"] = agi::ycbcr::Header(std::string(context->ass->GetScriptInfo("YCbCr Matrix"))).to_effective().to_existing_string();
	if (auto provider = context->project->VideoProvider()) {
		result["video_x"] = provider->GetWidth();
		result["video_y"] = provider->GetHeight();
		result["video_matrix"] = agi::ycbcr::Header(provider->GetRealColorSpace()).to_existing_string();
	}
	return encode(std::move(result));
}

std::string EditScriptInfo(agi::Context *context, json::Object const& request) {
	auto const& fields = static_cast<json::Object const&>(request.at("fields"));
	for (auto const& [key, element] : fields) {
		std::string const& value = element;
		if (!script_fields.count(key)) throw std::invalid_argument("unknown script property");
		check_single_line(value);
		if (key == "PlayResX" || key == "PlayResY" || key == "LayoutResX" || key == "LayoutResY") {
			if (!value.empty()) parse_integer(value, 0, 1000000);
		}
		if (key == "WrapStyle") parse_integer(value, 0, 3);
		if (key == "ScaledBorderAndShadow" && value != "yes" && value != "no")
			throw std::invalid_argument("ScaledBorderAndShadow must be yes or no");
		if (key == "YCbCr Matrix" && !agi::ycbcr::Header(value).valid())
			throw std::invalid_argument("invalid YCbCr Matrix");
	}
	bool changed = false;
	for (auto const& [key, element] : fields) {
		std::string const& value = element;
		if (context->ass->GetScriptInfo(key) == value) continue;
		context->ass->SetScriptInfo(key, value);
		changed = true;
	}
	if (changed) context->ass->Commit(_("property changes"), AssFile::COMMIT_SCRIPTINFO);
	return changed ? R"({"ok":true})" : R"({"ok":true,"unchanged":true})";
}

std::string StylePreview(agi::Context *context, json::Object const& request) {
	return style_preview(context, request);
}

std::string EditStyle(agi::Context *context, json::Object const& request) {
	std::string const& action = request.at("action");
	std::string const& source = request.at("source");
	auto existing = context->ass->GetStyle(source);
	if (action != "create" && !existing) throw std::invalid_argument("style no longer exists");
	if (action == "delete") {
		if (std::distance(context->ass->Styles.begin(), context->ass->Styles.end()) <= 1)
			throw std::invalid_argument("keep at least one style in the script");
		std::string const& replacement = request.at("replacement");
		if (!context->ass->GetStyle(replacement) || context->ass->GetStyle(replacement) == existing)
			throw std::invalid_argument("choose another style for existing references");
		StyleRenamer(context, existing->name, replacement).Replace();
		delete existing;
		context->ass->Commit(_("style delete"), AssFile::COMMIT_STYLES | AssFile::COMMIT_DIAG_FULL);
		return R"({"ok":true})";
	}
	if (action != "create" && action != "edit") throw std::invalid_argument("unknown style operation");
	AssStyle patch = existing ? *existing : AssStyle();
	auto const& fields = static_cast<json::Object const&>(request.at("fields"));
	for (auto const& [key, value] : fields) set_style_field(patch, key, static_cast<std::string const&>(value));
	auto collision = context->ass->GetStyle(patch.name);
	if (collision && (action == "create" || collision != existing))
		throw std::invalid_argument("a style with that name already exists");
	patch.UpdateData();
	if (action == "create") {
		context->ass->Styles.push_back(*new AssStyle(patch));
		context->ass->Commit(_("style copy"), AssFile::COMMIT_STYLES);
		return R"({"ok":true})";
	}
	if (patch.GetEntryData() == existing->GetEntryData()) return R"({"ok":true,"unchanged":true})";
	bool renamed = patch.name != existing->name;
	if (renamed) StyleRenamer(context, existing->name, patch.name).Replace();
	*existing = patch;
	existing->UpdateData();
	context->ass->Commit(_("style change"), AssFile::COMMIT_STYLES | (renamed ? AssFile::COMMIT_DIAG_FULL : 0));
	return R"({"ok":true})";
}
std::string StyleLibrary(agi::Context *context, json::Object const& request) {
	std::string const& action = request.at("action");
	std::string catalog;
	if (auto value = request.find("catalog"); value != request.end()) catalog = static_cast<std::string const&>(value->second);
	auto catalogs = AssStyleStorage::GetCatalogs();
	auto valid_catalog = [](std::string const& name) {
		check_single_line(name);
		if (name.empty() || name == "." || name == ".." || name.find_first_of("/\\:*?\"<>|") != name.npos ||
			name.find_first_not_of(" \t") == name.npos || name.back() == '.' || name.back() == ' ')
			throw std::invalid_argument("choose a valid catalog name");
	};
	auto found = std::find_if(catalogs.begin(), catalogs.end(), [&](auto const& name) { return boost::iequals(name, catalog); });
	if (action == "create_catalog") {
		valid_catalog(catalog);
		if (found != catalogs.end()) throw std::invalid_argument("a catalog with that name already exists");
		AssStyleStorage storage;
		storage.LoadCatalog(catalog);
		storage.Save();
		catalogs.push_back(catalog);
	}
	else if (!catalog.empty()) {
		valid_catalog(catalog);
		if (found == catalogs.end()) throw std::invalid_argument("style catalog no longer exists");
		catalog = *found;
	}
	else if (!catalogs.empty()) {
		auto selected = std::find(catalogs.begin(), catalogs.end(), context->ass->Properties.style_storage);
		catalog = selected == catalogs.end() ? catalogs.front() : *selected;
	}
	auto confirmed = [&] {
		auto value = request.find("confirmed");
		return value != request.end() && static_cast<json::Boolean const&>(value->second);
	};
	auto overwrite = [&] {
		auto value = request.find("overwrite");
		return value != request.end() && static_cast<json::Boolean const&>(value->second);
	};
	auto names = [&] {
		std::vector<std::string> values;
		for (auto const& value : static_cast<json::Array const&>(request.at("names"))) {
			std::string const& name = value;
			if (std::find(values.begin(), values.end(), name) == values.end()) values.push_back(name);
		}
		if (values.empty()) throw std::invalid_argument("select at least one style");
		return values;
	};
	if (action == "delete_catalog") {
		if (catalog.empty() || !confirmed()) throw std::invalid_argument("confirm catalog deletion first");
		if (catalogs.size() <= 1) throw std::invalid_argument("keep at least one catalog");
		agi::fs::Remove(config::path->Decode("?user/catalog/" + catalog + ".sty"));
		std::erase(catalogs, catalog);
		catalog = catalogs.front();
	}
	AssStyleStorage storage;
	if (!catalog.empty()) storage.LoadCatalog(catalog);

	auto copy_into_script = [&](std::vector<AssStyle> const& styles) {
		for (auto const& style : styles) {
			if (context->ass->GetStyle(style.name) && !overwrite())
				throw std::invalid_argument("a style already exists in this script; confirm replacement");
		}
		bool changed = false;
		for (auto const& style : styles) {
			if (auto existing = context->ass->GetStyle(style.name)) {
				if (existing->GetEntryData() == style.GetEntryData()) continue;
				*existing = style;
			}
			else context->ass->Styles.push_back(*new AssStyle(style));
			changed = true;
		}
		if (changed) context->ass->Commit(_("style import"), AssFile::COMMIT_STYLES);
	};
	std::unique_ptr<AssFile> imported;
	if (action == "import_preview" || action == "import") {
		std::string const& path = request.at("path");
		std::string encoding = CharSetDetect::GetEncoding(agi::fs::path(path));
		auto reader = SubtitleFormat::GetReader(agi::fs::path(path), encoding.c_str());
		if (!reader) throw std::invalid_argument("unsupported subtitle format");
		imported = std::make_unique<AssFile>();
		reader->ReadFile(imported.get(), agi::fs::path(path), context->project->Timecodes(), encoding.c_str());
		if (action == "import") {
			std::vector<AssStyle> styles;
			for (auto const& name : names()) {
				auto source = imported->GetStyle(name);
				if (!source) throw std::invalid_argument("import style no longer exists");
				styles.push_back(*source);
			}
			copy_into_script(styles);
		}
	}
	else if (action == "to_script") {
		std::vector<AssStyle> styles;
		for (auto const& name : names()) {
			auto source = storage.GetStyle(name);
			if (!source) throw std::invalid_argument("library style no longer exists");
			styles.push_back(*source);
		}
		copy_into_script(styles);
	}
	else if (action == "to_catalog") {
		if (catalog.empty()) throw std::invalid_argument("create or choose a catalog first");
		std::vector<AssStyle> styles;
		for (auto const& name : names()) {
			auto source = context->ass->GetStyle(name);
			if (!source) throw std::invalid_argument("script style no longer exists");
			if (storage.GetStyle(name) && !overwrite()) throw std::invalid_argument("a style already exists in this catalog; confirm replacement");
			styles.push_back(*source);
		}
		for (auto const& style : styles) {
			if (auto existing = storage.GetStyle(style.name)) *existing = style;
			else storage.push_back(std::make_unique<AssStyle>(style));
		}
		storage.Save();
	}
	else if (action == "edit" || action == "duplicate" || action == "delete") {
		if (catalog.empty()) throw std::invalid_argument("choose a catalog first");
		std::string const& source = request.at("source");
		auto existing = storage.GetStyle(source);
		if (!existing) throw std::invalid_argument("library style no longer exists");
		if (action == "delete") {
			if (!confirmed()) throw std::invalid_argument("confirm style deletion first");
			auto index = std::find_if(storage.begin(), storage.end(), [&](auto const& style) { return style.get() == existing; });
			storage.Delete(std::distance(storage.begin(), index));
		}
		else {
			AssStyle patch = *existing;
			for (auto const& [key, value] : static_cast<json::Object const&>(request.at("fields")))
				set_style_field(patch, key, static_cast<std::string const&>(value));
			auto collision = storage.GetStyle(patch.name);
			if (collision && (action == "duplicate" || collision != existing)) throw std::invalid_argument("a style with that name already exists");
			patch.UpdateData();
			if (action == "duplicate") storage.push_back(std::make_unique<AssStyle>(patch));
			else *existing = patch;
		}
		storage.Save();
	}
	else if (action != "list" && action != "create_catalog" && action != "delete_catalog")
		throw std::invalid_argument("unknown style library action");

	json::Object result;
	result["ok"] = true;
	result["catalog"] = catalog;
	json::Array catalog_names;
	for (auto const& name : catalogs) catalog_names.emplace_back(name);
	result["catalogs"] = std::move(catalog_names);
	json::Array styles;
	for (auto const& style : storage) styles.emplace_back(describe_style(*style));
	result["styles"] = std::move(styles);
	json::Array import_styles;
	if (imported) for (auto const& style : imported->Styles) import_styles.emplace_back(describe_style(style));
	result["import_styles"] = std::move(import_styles);
	return encode(std::move(result));
}

std::string Styling(agi::Context *context, json::Object const& request) {
	std::string const& action = request.at("action");
	auto active = context->selectionController->GetActiveLine();
	if (!active) throw std::invalid_argument("select a subtitle line");
	if (action == "commit" || action == "preview") {
		int64_t const id = static_cast<json::Integer const&>(request.at("id"));
		if (active->Id != id) throw std::invalid_argument("active line changed; choose a style for the current line");
		std::string const& name = request.at("style");
		auto style = context->ass->GetStyle(name);
		if (!style) throw std::invalid_argument("style does not exist");
		if (active->Style != style->name) {
			active->Style = style->name;
			context->ass->Commit(_("styling assistant"), AssFile::COMMIT_DIAG_META);
		}
		if (action == "commit") context->selectionController->NextLine();
	}
	else if (action == "next") context->selectionController->NextLine();
	else if (action == "prev") context->selectionController->PrevLine();
	else if (action != "start") throw std::invalid_argument("unknown styling action");
	if (auto seek = request.find("seek"); seek != request.end() && static_cast<json::Boolean const&>(seek->second))
		context->videoController->JumpToTime(context->selectionController->GetActiveLine()->Start);
	return R"({"ok":true})";
}

}
