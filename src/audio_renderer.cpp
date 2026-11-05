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

/// @file audio_renderer.cpp
/// @brief Base classes for audio renderers (spectrum, waveform, ...)
/// @ingroup audio_ui

#include "audio_renderer.h"

#include <libaegisub/audio/provider.h>

#include <algorithm>

namespace {
	template<typename T>
	bool compare_and_set(T &var, const T new_value)
	{
		if (var == new_value) return false;
		var = new_value;
		return true;
	}
}

AudioRendererCanvasCacheFactory::AudioRendererCanvasCacheFactory(AudioRenderer *renderer)
: renderer(renderer)
{
	assert(renderer);
}

std::unique_ptr<AudioCanvas> AudioRendererCanvasCacheFactory::ProduceBlock(int /* i */)
{
	auto canvas = std::make_unique<AudioCanvas>();
	canvas->Reset(renderer->cache_bitmap_width, renderer->pixel_height);
	return canvas;
}

size_t AudioRendererCanvasCacheFactory::GetBlockSize() const
{
	return sizeof(AudioCanvas) + static_cast<size_t>(renderer->cache_bitmap_width) * renderer->pixel_height * 4;
}

AudioRenderer::AudioRenderer()
{
	bitmaps.reserve(AudioStyle_MAX);
	for (int i = 0; i < AudioStyle_MAX; ++i)
		bitmaps.emplace_back(256, AudioRendererCanvasCacheFactory(this));

	// Make sure there's *some* values for those fields, and in the caches
	SetMillisecondsPerPixel(1);
	SetHeight(1);
}

void AudioRenderer::SetMillisecondsPerPixel(const double new_pixel_ms)
{
	if (compare_and_set(pixel_ms, new_pixel_ms))
	{
		if (renderer)
			renderer->SetMillisecondsPerPixel(pixel_ms);

		ResetBlockCount();
	}
}

void AudioRenderer::SetHeight(const int _pixel_height)
{
	if (compare_and_set(pixel_height, _pixel_height))
		Invalidate();
}

void AudioRenderer::SetAmplitudeScale(const float _amplitude_scale)
{
	if (compare_and_set(amplitude_scale, _amplitude_scale))
	{
		// A scaling of 0 or a negative scaling makes no sense
		assert(amplitude_scale > 0);
		if (renderer)
			renderer->SetAmplitudeScale(amplitude_scale);
		Invalidate();
	}
}

void AudioRenderer::SetRenderer(AudioRendererCanvasProvider *const _renderer)
{
	if (compare_and_set(renderer, _renderer))
	{
		Invalidate();

		if (renderer)
		{
			renderer->SetProvider(provider);
			renderer->SetAmplitudeScale(amplitude_scale);
			renderer->SetMillisecondsPerPixel(pixel_ms);
		}
	}
}

void AudioRenderer::SetAudioProvider(agi::AudioProvider *const _provider)
{
	if (compare_and_set(provider, _provider))
	{
		Invalidate();

		if (renderer)
			renderer->SetProvider(provider);

		ResetBlockCount();
	}
}

void AudioRenderer::SetCacheMaxSize(const size_t max_size)
{
	// Limit the bitmap cache sizes to 16 MB hard, to avoid the risk of exhausting
	// system bitmap object resources and similar. Experimenting shows that 16 MB
	// bitmap cache should be plenty even if working with a one hour audio clip.
	cache_bitmap_maxsize = std::min<size_t>(max_size/8, 0x1000000);
	// The renderer gets whatever is left.
	cache_renderer_maxsize = max_size - 4*cache_bitmap_maxsize;
}

void AudioRenderer::ResetBlockCount()
{
	if (provider)
	{
		const size_t total_blocks = NumBlocks(provider->GetNumSamples());
		for (auto& bmp : bitmaps) bmp.SetBlockCount(total_blocks);
	}
}

size_t AudioRenderer::NumBlocks(const int64_t samples) const
{
	const double duration = samples * 1000.0 / provider->GetSampleRate();
	return static_cast<size_t>(duration / pixel_ms / cache_bitmap_width);
}

AudioCanvas const &AudioRenderer::GetCachedCanvas(const int i, const AudioRenderingStyle style)
{
	assert(provider);
	assert(renderer);

	bool created = false;
	auto& canvas = bitmaps[style].Get(i, &created);
	if (created)
	{
		canvas.Reset(cache_bitmap_width, pixel_height);
		renderer->Render(canvas, i*cache_bitmap_width, style);
		needs_age = true;
	}

	return canvas;
}

void AudioRenderer::Render(AudioCanvas &canvas, const int start, const AudioRenderingStyle style)
{
	assert(start >= 0);

	if (!provider) return;
	if (!renderer) return;

	const int length = canvas.Width();
	if (length <= 0) return;

	// One past last absolute pixel strip to render
	const int end = start + length;
	// Figure out which range of bitmaps are required
	const int firstbitmap = start / cache_bitmap_width;
	// And the offset in it to start its use at
	const int firstbitmapoffset = start % cache_bitmap_width;
	// The last bitmap required
	const int lastbitmap = std::min<int>(end / cache_bitmap_width, NumBlocks(provider->GetDecodedSamples()) - 1);

	// Blocks are drawn one after another, clipped by the canvas itself.
	int x = -firstbitmapoffset;
	for (int i = firstbitmap; i <= lastbitmap; ++i)
	{
		canvas.Blit(GetCachedCanvas(i, style), x, 0);
		x += cache_bitmap_width;
	}

	// Now render blank audio from where the blocks ended to the right edge
	if (x < length)
		renderer->RenderBlank(canvas, x, length - x, style);

	if (needs_age)
	{
		bitmaps[style].Age(cache_bitmap_maxsize);
		renderer->AgeCache(cache_renderer_maxsize);
		needs_age = false;
	}
}

void AudioRenderer::Invalidate()
{
	for (auto& bmp : bitmaps) bmp.Age(0);
	needs_age = false;
}

void AudioRendererCanvasProvider::SetProvider(agi::AudioProvider *const _provider)
{
	if (compare_and_set(provider, _provider))
		OnSetProvider();
}

void AudioRendererCanvasProvider::SetMillisecondsPerPixel(const double new_pixel_ms)
{
	if (compare_and_set(pixel_ms, new_pixel_ms))
		OnSetMillisecondsPerPixel();
}

void AudioRendererCanvasProvider::SetAmplitudeScale(const float _amplitude_scale)
{
	if (compare_and_set(amplitude_scale, _amplitude_scale))
		OnSetAmplitudeScale();
}
