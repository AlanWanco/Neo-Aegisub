// Copyright (c) 2006, Rodrigo Braz Monteiro
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

#include "timing_processor.h"
#include "localisation.h"

#include "ass_dialogue.h"
#include "ass_file.h"
#include "async_video_provider.h"
#include "include/aegisub/context.h"
#include "project.h"
#include "selection_controller.h"

#include <algorithm>
#include <cmath>
#include <stdexcept>
#include <utility>
#include <vector>

namespace {
static int get_closest_kf(std::vector<int> const& kf, int frame) {
	const auto pos = std::upper_bound(kf.begin(), kf.end(), frame);
	// Return last keyframe if this is after the last one
	if (pos == end(kf)) return kf.back();
	// *pos is greater than frame, and *(pos - 1) is less than or equal to frame
	return (pos == begin(kf) || *pos - frame < frame - *(pos - 1)) ? *pos : *(pos - 1);
}

template<class Iter, class Field>
static int safe_time(Iter begin, Iter end, AssDialogue *comp, int initial, Field field, int const& (*cmp)(int const&, int const&)) {
	// Compare to every previous line (yay for O(n^2)!) to see if it's OK to add lead-in
	for (; begin != end; ++begin) {
		// If the line doesn't already collide with this line, extend it only
		// to the edge of the line
		if (!comp->CollidesWith(*begin))
			initial = cmp(initial, (*begin)->*field);
	}
	return initial;
}

}

std::size_t ProcessTiming(agi::Context *context, TimingProcessorSettings const& settings) {
	for (int value : {settings.lead_in, settings.lead_out, settings.before_start, settings.after_start, settings.before_end, settings.after_end, settings.adjacent_gap, settings.adjacent_overlap}) {
		if (value < 0 || value > 35999990) throw std::invalid_argument("timing thresholds must be between 0 and 35999990 ms");
	}
	if (!std::isfinite(settings.adjacent_bias) || settings.adjacent_bias < 0 || settings.adjacent_bias > 1)
		throw std::invalid_argument("adjacent bias must be between 0 and 1");
	if (settings.keyframes && (context->project->Keyframes().empty() || !context->project->Timecodes().IsLoaded()))
		throw std::invalid_argument("keyframe snapping requires loaded keyframes and timecodes");
	for (auto const& name : settings.styles) {
		if (!context->ass->GetStyle(name)) throw std::invalid_argument("selected style no longer exists");
	}
	std::vector<AssDialogue *> sorted;
	for (auto& line : context->ass->Events) {
		if (line.Comment || !settings.styles.count(line.Style.get())) continue;
		if (settings.only_selection && !context->selectionController->GetSelectedSet().count(&line)) continue;
		if (line.Start > line.End) throw std::invalid_argument("line " + std::to_string(line.Row + 1) + " has negative duration");
		sorted.push_back(&line);
	}
	std::sort(sorted.begin(), sorted.end(), [](auto a, auto b) { return a->Start < b->Start; });
	if (sorted.empty()) return 0;
	std::vector<std::pair<int, int>> original;
	for (auto line : sorted) original.emplace_back(line->Start, line->End);

	// Add lead-in/out
	if (settings.has_lead_in && settings.lead_in) {
		for (size_t i = 0; i < sorted.size(); ++i)
			sorted[i]->Start = safe_time(sorted.rend() - i, sorted.rend(),
				sorted[i], sorted[i]->Start - settings.lead_in,
				&AssDialogue::End, &std::max<int>);
	}

	if (settings.has_lead_out && settings.lead_out) {
		for (size_t i = 0; i < sorted.size(); ++i)
			sorted[i]->End = safe_time(sorted.begin() + i + 1, sorted.end(),
				sorted[i], sorted[i]->End + settings.lead_out,
				&AssDialogue::Start, &std::min<int>);
	}

	// Make adjacent
	if (settings.adjacent) {
		double bias = settings.adjacent_bias;

		for (size_t i = 1; i < sorted.size(); ++i) {
			AssDialogue *prev = sorted[i - 1];
			AssDialogue *cur = sorted[i];

			int dist = cur->Start - prev->End;
			if ((dist < 0 && -dist <= settings.adjacent_overlap) || (dist > 0 && dist <= settings.adjacent_gap)) {
				int setPos = prev->End + int(dist * bias);
				cur->Start = setPos;
				prev->End = setPos;
			}
		}
	}

	// Keyframe snapping
	if (settings.keyframes) {
		std::vector<int> kf = context->project->Keyframes();
		auto fps = context->project->Timecodes();
		if (auto provider = context->project->VideoProvider())
			kf.push_back(provider->GetFrameCount() - 1);

		for (AssDialogue *cur : sorted) {
			// Get start/end frames
			int startF = fps.FrameAtTime(cur->Start, agi::vfr::START);
			int endF = fps.FrameAtTime(cur->End, agi::vfr::END);

			// Get closest for start
			int closest = get_closest_kf(kf, startF);
			int time = fps.TimeAtFrame(closest, agi::vfr::START);
			if ((closest > startF && time - cur->Start <= settings.before_start) || (closest < startF && cur->Start - time <= settings.after_start))
				cur->Start = time;

			// Get closest for end
			closest = get_closest_kf(kf, endF) - 1;
			time = fps.TimeAtFrame(closest, agi::vfr::END);
			if ((closest > endF && time - cur->End <= settings.before_end) || (closest < endF && cur->End - time <= settings.after_end))
				cur->End = time;
		}
	}

	size_t changed = 0;
	for (size_t i = 0; i < sorted.size(); ++i)
		if (sorted[i]->Start != original[i].first || sorted[i]->End != original[i].second) ++changed;
	if (changed) context->ass->Commit(_("timing processor"), AssFile::COMMIT_DIAG_TIME);
	return changed;
}
