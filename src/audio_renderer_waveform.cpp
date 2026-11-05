// Copyright (c) 2010, Niels Martin Hansen
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

#include "audio_renderer_waveform.h"

#include "audio_colorscheme.h"
#include "options.h"

#include <libaegisub/audio/provider.h>

#include <algorithm>
#include <cstdint>

enum {
	/// Only render the peaks
	Waveform_MaxOnly = 0,
	/// Render the peaks and averages
	Waveform_MaxAvg,
	Waveform_Continuous
};

AudioWaveformRenderer::AudioWaveformRenderer(std::string const& color_scheme_name)
: render_averages(OPT_GET("Audio/Display/Waveform Style")->GetInt() == Waveform_MaxAvg)
{
	colors.reserve(AudioStyle_MAX);
	for (int i = 0; i < AudioStyle_MAX; ++i)
		colors.emplace_back(6, color_scheme_name, i);
}

AudioWaveformRenderer::~AudioWaveformRenderer() { }

void AudioWaveformRenderer::Render(AudioCanvas &canvas, int start, AudioRenderingStyle style)
{
	const int width = canvas.Width();
	const int height = canvas.Height();
	int midpoint = height / 2;

	const AudioColorScheme *pal = &colors[style];

	double pixel_samples = pixel_ms * provider->GetSampleRate() / 1000.0;

	// Fill the background
	canvas.Clear(pal->rgb(0.0f));

	// Make sure we've got a buffer to fill with audio data
	if (!audio_buffer)
	{
		// Buffer for one pixel strip of audio
		size_t buffer_needed = pixel_samples * provider->GetChannels() * provider->GetBytesPerSample();
		audio_buffer.reset(new char[buffer_needed]);
	}

	double cur_sample = start * pixel_samples;

	assert(provider->GetBytesPerSample() == 2);
	assert(provider->GetChannels() == 1);

	uint32_t const peak_rgb = pal->rgb(0.4f);
	uint32_t const avg_rgb = pal->rgb(0.7f);

	for (int x = 0; x < width; ++x)
	{
		provider->GetAudio(audio_buffer.get(), (int64_t)cur_sample, (int64_t)pixel_samples);
		cur_sample += pixel_samples;

		int peak_min = 0, peak_max = 0;
		int64_t avg_min_accum = 0, avg_max_accum = 0;
		auto aud = reinterpret_cast<const int16_t *>(audio_buffer.get());
		for (int si = pixel_samples; si > 0; --si, ++aud)
		{
			if (*aud > 0)
			{
				peak_max = std::max(peak_max, (int)*aud);
				avg_max_accum += *aud;
			}
			else
			{
				peak_min = std::min(peak_min, (int)*aud);
				avg_min_accum += *aud;
			}
		}

		// midpoint is half height
		peak_min = std::max((int)(peak_min * amplitude_scale * midpoint) / 0x8000, -midpoint);
		peak_max = std::min((int)(peak_max * amplitude_scale * midpoint) / 0x8000, midpoint);
		int avg_min = std::max((int)(avg_min_accum * amplitude_scale * midpoint / pixel_samples) / 0x8000, -midpoint);
		int avg_max = std::min((int)(avg_max_accum * amplitude_scale * midpoint / pixel_samples) / 0x8000, midpoint);

		canvas.VerticalLine(x, midpoint - peak_max, midpoint - peak_min, peak_rgb);
		if (render_averages)
			canvas.VerticalLine(x, midpoint - avg_max, midpoint - avg_min, avg_rgb);
	}

	// Horizontal zero-point line
	canvas.HorizontalLine(midpoint, 0, width - 1,
		render_averages ? pal->rgb(1.0f) : peak_rgb);
}

void AudioWaveformRenderer::RenderBlank(AudioCanvas &canvas, int x, int width, AudioRenderingStyle style)
{
	const AudioColorScheme *pal = &colors[style];
	uint32_t const line = pal->rgb(1.0f);
	uint32_t const bg = pal->rgb(0.0f);

	// Draw the line as background above and below, and line in the middle, to avoid
	// overdraw flicker (the common theme in all of audio display direct drawing).
	int const height = canvas.Height();
	int halfheight = height / 2;

	canvas.FillRect(x, 0, width, halfheight, bg);
	canvas.FillRect(x, halfheight + 1, width, height - halfheight - 1, bg);
	canvas.HorizontalLine(halfheight, x, x + width - 1, line);
}
