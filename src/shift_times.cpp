// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

#include "shift_times.h"
#include "localisation.h"

#include "ass_dialogue.h"
#include "ass_file.h"
#include "include/aegisub/context.h"
#include "options.h"
#include "project.h"
#include "selection_controller.h"
#include "subs_controller.h"

#include <libaegisub/cajun/reader.h>
#include <libaegisub/cajun/writer.h>
#include <libaegisub/fs.h>
#include <libaegisub/io.h>
#include <libaegisub/log.h>
#include <libaegisub/path.h>
#include <libaegisub/vfr.h>

#include <limits>
#include <stdexcept>
#include <vector>

json::Array LoadShiftTimesHistory() {
	try {
		json::UnknownElement root;
		json::Reader::Read(root, *agi::io::Open(config::path->Decode("?user/shift_history.json")));
		return std::move(static_cast<json::Array&>(root));
	}
	catch (agi::fs::FileSystemError const& e) {
		LOG_D("shift_times/load_history") << e.GetMessage();
	}
	catch (json::Exception const& e) {
		LOG_D("shift_times/load_history") << e.what();
	}
	return {};
}

void ClearShiftTimesHistory() {
	agi::fs::Remove(config::path->Decode("?user/shift_history.json"));
}

std::size_t ShiftTimes(agi::Context *context, ShiftTimesSettings const& settings) {
	if (settings.amount < 0 || settings.amount > 35999990 || settings.mode < 0 || settings.mode > 2 || settings.fields < 0 || settings.fields > 2)
		throw std::invalid_argument("invalid shift settings");
	auto const& fps = context->project->Timecodes();
	if (!settings.by_time && !fps.IsLoaded())
		throw std::invalid_argument("frame shifting requires video or timecodes");
	auto const& selected = context->selectionController->GetSelectedSet();
	if (settings.mode && selected.empty())
		throw std::invalid_argument("select subtitle rows before shifting them");
	if (!settings.amount) return 0;

	int const shift = settings.backward ? -settings.amount : settings.amount;
	auto shifted = [&](int time, agi::vfr::Time type) {
		if (settings.by_time) return agi::Time(time + shift);
		int64_t const frame = static_cast<int64_t>(fps.FrameAtTime(time, type)) + shift;
		if (frame < std::numeric_limits<int>::min() || frame > std::numeric_limits<int>::max())
			throw std::invalid_argument("shifted frame is outside the supported range");
		return agi::Time(fps.TimeAtFrame(static_cast<int>(frame), type));
	};
	struct Change { AssDialogue *line; agi::Time start; agi::Time end; };
	std::vector<Change> changes;
	json::Array ranges;
	int block_start = 0;
	int last_row = 0;
	bool onward = false;
	// Calculate every result first, so a rejected conversion never changes some rows.
	for (auto& line : context->ass->Events) {
		bool const in_selection = selected.count(&line);
		if (in_selection) {
			onward = true;
			if (!block_start) block_start = line.Row + 1;
		}
		else if (block_start) {
			json::Object range;
			range["start"] = block_start;
			range["end"] = line.Row;
			ranges.emplace_back(std::move(range));
			block_start = 0;
		}
		last_row = line.Row + 1;
		if ((settings.mode == 1 && !in_selection) || (settings.mode == 2 && !onward)) continue;
		agi::Time const start = settings.fields == 2 ? line.Start : shifted(line.Start, agi::vfr::START);
		agi::Time const end = settings.fields == 1 ? line.End : shifted(line.End, agi::vfr::END);
		if (start != line.Start || end != line.End) changes.push_back({&line, start, end});
	}
	if (block_start) {
		json::Object range;
		range["start"] = block_start;
		range["end"] = last_row;
		ranges.emplace_back(std::move(range));
	}
	for (auto const& change : changes) {
		change.line->Start = change.start;
		change.line->End = change.end;
	}
	if (!changes.empty()) context->ass->Commit(_("shifting"), AssFile::COMMIT_DIAG_TIME);

	OPT_SET("Tool/Shift Times/ByTime")->SetBool(settings.by_time);
	OPT_SET("Tool/Shift Times/Direction")->SetBool(settings.backward);
	OPT_SET("Tool/Shift Times/Type")->SetInt(settings.fields);
	OPT_SET("Tool/Shift Times/Affect")->SetInt(settings.mode);
	OPT_SET(settings.by_time ? "Tool/Shift Times/Time" : "Tool/Shift Times/Frames")->SetInt(settings.amount);

	json::Object entry;
	entry["filename"] = context->subsController->Filename().filename().string();
	entry["is by time"] = settings.by_time;
	entry["is backward"] = settings.backward;
	entry["amount"] = settings.by_time ? agi::Time(settings.amount).GetAssFormatted() : std::to_string(settings.amount);
	entry["fields"] = settings.fields;
	entry["mode"] = settings.mode;
	entry["selection"] = std::move(ranges);
	auto history = LoadShiftTimesHistory();
	history.insert(history.begin(), std::move(entry));
	if (history.size() > 50) history.resize(50);
	try {
		agi::JsonWriter::Write(history, agi::io::Save(config::path->Decode("?user/shift_history.json")).Get());
	}
	catch (agi::fs::FileSystemError const& e) {
		LOG_E("shift_times/save_history") << e.GetMessage();
	}
	return changes.size();
}
