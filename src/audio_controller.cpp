// Copyright (c) 2009-2010, Niels Martin Hansen
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

#include "audio_controller.h"

#include "audio_timing.h"
#include "include/aegisub/audio_player.h"
#include "include/aegisub/context.h"
#include "options.h"
#include "project.h"

#include <libaegisub/audio/provider.h>
#include <libaegisub/audio/provider_rate.h>

#include <algorithm>
#include <cmath>

AudioController::AudioController(agi::Context *context)
: context(context)
, provider_connection(context->project->AddAudioProviderListener(&AudioController::OnAudioProvider, this))
{
	playback_timer.on_tick = [this] { OnPlaybackTimer(); };

	BindConnection(OPT_SUB("Audio/Player", &AudioController::OnAudioPlayerChanged, this));
}

AudioController::~AudioController()
{
	Stop();
}

void AudioController::OnPlaybackTimer()
{
	if (!player) return;

	int64_t pos = player->GetCurrentPosition();
	if (!player->IsPlaying() ||
		(playback_mode != PM_ToEnd && pos >= player->GetEndPosition()+200))
	{
		// The +200 is to allow the player to end the sound output cleanly,
		// otherwise a popping artifact can sometimes be heard.
		Stop();
	}
	else
	{
		AnnouncePlaybackPosition(MillisecondsFromSamples(pos));
	}
}

void AudioController::OnAudioPlayerChanged()
{
	if (!provider) return;

	Stop();
	player.reset();
	playback_provider.reset();

	try
	{
		playback_provider = std::make_unique<agi::PlaybackAudioProvider>(provider, playback_rate);
		player = AudioPlayerFactory::GetAudioPlayer(playback_provider.get(), nullptr);
		player->SetVolume(volume);
	}
	catch (...)
	{
		/// @todo This really shouldn't be just swallowing all audio player open errors
		context->project->CloseAudio();
	}
	AnnounceAudioPlayerOpened();
}

void AudioController::OnAudioProvider(agi::AudioProvider *new_provider)
{
	Stop();
	player.reset();
	playback_provider.reset();
	provider = new_provider;
	OnAudioPlayerChanged();
}

void AudioController::SetTimingController(std::unique_ptr<AudioTimingController> new_controller)
{
	timing_controller = std::move(new_controller);
	if (timing_controller)
		timing_controller_connection = timing_controller->AddUpdatedPrimaryRangeListener(&AudioController::OnTimingControllerUpdatedPrimaryRange, this);
	else
		timing_controller_connection = agi::signal::Connection();

	AnnounceTimingControllerChanged();
}

void AudioController::OnTimingControllerUpdatedPrimaryRange()
{
	if (playback_mode == PM_PrimaryRange)
		player->SetEndPosition(SamplesFromMilliseconds(timing_controller->GetPrimaryPlaybackRange().end()));
}

void AudioController::PlayRange(const TimeRange &range)
{
	if (!player) return;

	int64_t start = SamplesFromMilliseconds(range.begin());
	int64_t end = SamplesFromMilliseconds(range.end());
	player->Play(start, std::max<int64_t>(0, end - start));
	playback_mode = PM_Range;
	playback_timer.Start(20);

	AnnouncePlaybackPosition(range.begin());
}

void AudioController::PlayPrimaryRange()
{
	PlayRange(GetPrimaryPlaybackRange());
	if (playback_mode == PM_Range)
		playback_mode = PM_PrimaryRange;
}

void AudioController::PlayToEndOfPrimary(int start_ms)
{
	PlayRange(TimeRange(start_ms, GetPrimaryPlaybackRange().end()));
	if (playback_mode == PM_Range)
		playback_mode = PM_PrimaryRange;
}

void AudioController::PlayToEnd(int start_ms)
{
	if (!player) return;

	int64_t start_sample = SamplesFromMilliseconds(start_ms);
	player->Play(start_sample, std::max<int64_t>(0, playback_provider->GetNumSamples() - start_sample));
	playback_mode = PM_ToEnd;
	playback_timer.Start(20);

	AnnouncePlaybackPosition(start_ms);
}

void AudioController::Stop()
{
	if (!player) return;

	player->Stop();
	playback_mode = PM_NotPlaying;
	playback_timer.Stop();

	AnnouncePlaybackStop();
}

bool AudioController::IsPlaying()
{
	return player && playback_mode != PM_NotPlaying;
}

int AudioController::GetPlaybackPosition()
{
	if (!IsPlaying()) return 0;

	return MillisecondsFromSamples(player->GetCurrentPosition());
}

int AudioController::GetDuration() const
{
	if (!provider) return 0;
	return (provider->GetNumSamples() * 1000 + provider->GetSampleRate() - 1) / provider->GetSampleRate();
}

TimeRange AudioController::GetPrimaryPlaybackRange() const
{
	if (timing_controller)
		return timing_controller->GetPrimaryPlaybackRange();
	else
		return TimeRange{0, 0};
}

void AudioController::SetVolume(double volume)
{
	this->volume = volume;
	if (player) player->SetVolume(volume);
}

void AudioController::SetPlaybackRate(double rate, int source_position_ms)
{
	if (!std::isfinite(rate) || rate < 0.5 || rate > 2.0 || rate == playback_rate) return;

	auto mode = playback_mode;
	int position = source_position_ms >= 0 ? source_position_ms : GetPlaybackPosition();
	int end = player ? MillisecondsFromSamples(player->GetEndPosition()) : 0;
	playback_rate = rate;
	OnAudioPlayerChanged();
	if (!player || mode == PM_NotPlaying) return;

	if (mode == PM_ToEnd)
		PlayToEnd(position);
	else {
		PlayRange(TimeRange(position, std::max(position, end)));
		playback_mode = mode;
	}
}

int64_t AudioController::SamplesFromMilliseconds(int64_t ms) const
{
	if (!provider || !playback_provider) return 0;
	auto source_sample = (ms * provider->GetSampleRate() + 999) / 1000;
	return playback_provider->SourceToPlayback(source_sample);
}

int64_t AudioController::MillisecondsFromSamples(int64_t samples) const
{
	if (!provider || !playback_provider) return 0;
	return playback_provider->PlaybackToSource(samples) * 1000 / provider->GetSampleRate();
}
