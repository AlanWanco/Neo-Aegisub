// Copyright (c) 2005-2007, Rodrigo Braz Monteiro
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

#include "video_controller.h"

#include "ass_dialogue.h"
#include "ass_file.h"
#include "audio_controller.h"
#include "compat.h"
#include "format.h"
#include "include/aegisub/context.h"
#include "options.h"
#include "project.h"
#include "selection_controller.h"
#include "time_range.h"
#include "async_video_provider.h"
#include "utils.h"

#include <libaegisub/ass/time.h>

#include <cmath>
#include <limits>
#include <wx/log.h>

VideoController::VideoController(agi::Context *c)
: context(c)
, playAudioOnStep(OPT_GET("Audio/Plays When Stepping Video"))
, connections(agi::signal::make_vector({
	context->ass->AddCommitListener(&VideoController::OnSubtitlesCommit, this),
	context->project->AddVideoProviderListener(&VideoController::OnNewVideoProvider, this),
	context->selectionController->AddActiveLineListener(&VideoController::OnActiveLineChanged, this),
}))
{
	Bind(EVT_VIDEO_ERROR, &VideoController::OnVideoError, this);
	Bind(EVT_SUBTITLES_ERROR, &VideoController::OnSubtitlesError, this);
	playback.Bind(wxEVT_TIMER, &VideoController::OnPlayTimer, this);
}

void VideoController::OnNewVideoProvider(AsyncVideoProvider *new_provider) {
	Stop();
	provider = new_provider;
	color_matrix = std::nullopt;
}

void VideoController::OnSubtitlesCommit(int type, const AssDialogue *changed) {
	if (!provider) return;

	if ((type & AssFile::COMMIT_SCRIPTINFO) || type == AssFile::COMMIT_NEW) {
		auto new_matrix = context->ass->GetYCbCrMatrix();
		if (new_matrix != color_matrix) {
			color_matrix = new_matrix;
			provider->SetColorSpace(new_matrix);
		}
	}

	if (!changed)
		provider->LoadSubtitles(context->ass.get());
	else
		provider->UpdateSubtitles(changed);
}

void VideoController::OnActiveLineChanged(AssDialogue *line) {
	if (line && provider && OPT_GET("Video/Subtitle Sync")->GetBool()) {
		HaltPlayback();
		JumpToTime(line->Start);
	}
}

void VideoController::RequestFrame() {
	context->ass->Properties.video_position = frame_n;
	// Keep a little history: the only request that survives is the newest one,
	// but a frame can still arrive for a request a tick or two old.
	request_times[frame_n] = std::chrono::steady_clock::now();
	while (request_times.size() > 8) request_times.erase(request_times.begin());
	provider->RequestFrame(frame_n, TimeAtFrame(frame_n));
}

void VideoController::NoteFrameArrived(int frame_number) {
	auto const request = request_times.find(frame_number);
	if (request == request_times.end()) return;
	using namespace std::chrono;
	double const measured = duration<double, std::milli>(steady_clock::now() - request->second).count();
	// Anything older will not arrive now that this one has.
	request_times.erase(request_times.begin(), std::next(request));
	// Smooth hard: a single slow frame must not make the picture jump ahead of
	// the sound, and the estimate only ever shifts the request, not the clock.
	frame_latency_ms = frame_latency_ms * .7 + mid(0., measured, 120.) * .3;
}

void VideoController::JumpToFrame(int n) {
	if (!provider) return;

	bool was_playing = IsPlaying();
	if (was_playing)
		HaltPlayback();

	frame_n = mid(0, n, provider->GetFrameCount() - 1);
	RequestFrame();
	Seek(frame_n);

	if (was_playing)
		Play();
}

void VideoController::JumpToTime(int ms, agi::vfr::Time end) {
	JumpToFrame(FrameAtTime(ms, end));
}

void VideoController::NextFrame() {
	if (!provider || IsPlaying() || frame_n == provider->GetFrameCount())
		return;

	JumpToFrame(frame_n + 1);
	if (playAudioOnStep->GetBool())
		context->audioController->PlayRange(TimeRange(TimeAtFrame(frame_n - 1), TimeAtFrame(frame_n)));
}

void VideoController::PrevFrame() {
	if (!provider || IsPlaying() || frame_n == 0)
		return;

	JumpToFrame(frame_n - 1);
	if (playAudioOnStep->GetBool())
		context->audioController->PlayRange(TimeRange(TimeAtFrame(frame_n), TimeAtFrame(frame_n + 1)));
}

void VideoController::Play() {
	if (IsPlaying()) {
		Stop();
		return;
	}

	if (!provider) return;

	start_ms = TimeAtFrame(frame_n);
	end_frame = provider->GetFrameCount() - 1;
	loop_start_ms = 0;
	loop_end_ms = -1;

	context->audioController->PlayToEnd(start_ms);

	playback_start_time = std::chrono::steady_clock::now();
	playback.Start(10);
}

void VideoController::PlayLine() {
	HaltPlayback();

	AssDialogue *curline = context->selectionController->GetActiveLine();
	if (!curline) return;

	context->audioController->PlayRange(TimeRange(curline->Start, curline->End));

	// Round-trip conversion to convert start to exact
	int startFrame = FrameAtTime(context->selectionController->GetActiveLine()->Start, agi::vfr::START);
	start_ms = TimeAtFrame(startFrame);
	end_frame = FrameAtTime(context->selectionController->GetActiveLine()->End, agi::vfr::END) + 1;
	loop_start_ms = start_ms;
	loop_end_ms = curline->End;

	JumpToFrame(startFrame);

	playback_start_time = std::chrono::steady_clock::now();
	playback.Start(10);
}

void VideoController::Stop() {
	if (!IsPlaying()) return;
	// The frame asked for last is the one the audio will have reached by the
	// time it finished decoding, so pausing there would leave the picture a
	// couple of frames early. Stop where the sound actually stopped.
	double const stopped_at = GetPlaybackTime();
	HaltPlayback();
	int const frame = FrameAtTime(static_cast<int>(stopped_at));
	if (frame != frame_n) {
		frame_n = frame;
		RequestFrame();
		Seek(frame_n);
	}
}

void VideoController::HaltPlayback() {
	if (IsPlaying()) {
		playback.Stop();
		context->audioController->Stop();
	}
}

void VideoController::SetPlaybackRate(double rate) {
	if (!std::isfinite(rate) || rate < 0.5 || rate > 2.0 || rate == playback_rate) return;
	if (IsPlaying()) {
		auto now = std::chrono::steady_clock::now();
		auto elapsed = std::chrono::duration<double, std::milli>(now - playback_start_time).count();
		start_ms += static_cast<int>(std::llround(elapsed * playback_rate));
		playback_start_time = now;
	}
	playback_rate = rate;
	// Use the same source-time anchor for both streams when video is playing.
	context->audioController->SetPlaybackRate(rate, IsPlaying() ? start_ms : -1);
	if (IsPlaying()) playback_start_time = std::chrono::steady_clock::now();
	PlaybackRateChange(rate);
}

double VideoController::GetPlaybackTime() const {
    if (!IsPlaying()) return TimeAtFrame(frame_n);
    if (context->audioController->IsPlaying())
        return context->audioController->GetPlaybackPosition();

    using namespace std::chrono;
    double elapsed = duration<double, std::milli>(steady_clock::now() - playback_start_time).count();
    return start_ms + elapsed * playback_rate;
}

void VideoController::OnPlayTimer(wxTimerEvent &) {
    using namespace std::chrono;
    // While audio plays it is the master clock. Re-anchor the video-only clock to it so
    // that if audio stops early (range end) video continues from the last heard position.
    if (context->audioController->IsPlaying()) {
        start_ms = context->audioController->GetPlaybackPosition();
        playback_start_time = steady_clock::now();
    }
    double time = std::min<double>(GetPlaybackTime(), std::numeric_limits<int>::max());
	int const clock_frame = FrameAtTime(static_cast<int>(time));
	// Decoding takes a while, and the picture is displayed the moment it lands.
	// Asking for the frame the audio will have reached by then keeps the two
	// together; without it the picture trails the sound by the decoding time.
	int next_frame = clock_frame;
	if (context->audioController->IsPlaying() && loop_end_ms < 0)
		next_frame = FrameAtTime(static_cast<int>(time + frame_latency_ms));
	if (next_frame == frame_n) return;

	// The end of the range is decided by the clock, not by the frame asked for
	// ahead of it, or playback would stop early by the lookahead.
	if (clock_frame >= end_frame) {
		if (!OPT_GET("Video/Loop Playback")->GetBool()) {
			Stop();
			return;
		}
		start_ms = loop_start_ms;
		if (loop_end_ms < 0)
			context->audioController->PlayToEnd(start_ms);
		else
			context->audioController->PlayRange(TimeRange(start_ms, loop_end_ms));
		playback_start_time = steady_clock::now();
		frame_n = FrameAtTime(start_ms);
		RequestFrame();
		Seek(frame_n);
	}
	else {
		frame_n = next_frame;
		RequestFrame();
		Seek(frame_n);
	}
}

double VideoController::GetARFromType(AspectRatio type) const {
	switch (type) {
		case AspectRatio::Default:    return (double)provider->GetWidth()/provider->GetHeight();
		case AspectRatio::Fullscreen: return 4.0/3.0;
		case AspectRatio::Widescreen: return 16.0/9.0;
		case AspectRatio::Cinematic:  return 2.35;
        default: throw agi::InternalError("Bad AR type");
	}
}

void VideoController::SetAspectRatio(double value) {
	ar_type = AspectRatio::Custom;
	ar_value = mid(.5, value, 5.);
	context->ass->Properties.ar_mode = (int)ar_type;
	context->ass->Properties.ar_value = ar_value;
	ARChange(ar_type, ar_value);
}

void VideoController::SetAspectRatio(AspectRatio type) {
	ar_value = mid(.5, GetARFromType(type), 5.);
	ar_type = type;
	context->ass->Properties.ar_mode = (int)ar_type;
	context->ass->Properties.ar_value = ar_value;
	ARChange(ar_type, ar_value);
}

int VideoController::TimeAtFrame(int frame, agi::vfr::Time type) const {
	return context->project->Timecodes().TimeAtFrame(frame, type);
}

int VideoController::FrameAtTime(int time, agi::vfr::Time type) const {
	return context->project->Timecodes().FrameAtTime(time, type);
}

void VideoController::OnVideoError(VideoProviderErrorEvent const& err) {
	wxLogError(
		fmt_tl("Failed seeking video. The video file may be corrupt or incomplete.\nError message reported: %s",
		err.GetMessage()));
}

void VideoController::OnSubtitlesError(SubtitlesProviderErrorEvent const& err) {
	wxLogError(
		fmt_tl("Failed rendering subtitles.\nError message reported: %s",
		err.GetMessage()));
}
