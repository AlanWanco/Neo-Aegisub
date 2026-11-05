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

#pragma once

#include <cstddef>
#include <memory>
#include <vector>

#include "audio_canvas.h"
#include "audio_rendering_style.h"
#include "block_cache.h"

class AudioRenderer;
class AudioRendererCanvasProvider;
namespace agi { class AudioProvider; }

/// @class AudioRendererCanvasCacheFactory
/// @brief Produces AudioCanvas blocks for DataBlockCache storage
struct AudioRendererCanvasCacheFactory {
	typedef std::unique_ptr<AudioCanvas> BlockType;

	/// The audio renderer we're producing canvases for
	AudioRenderer *renderer;

	/// @brief Constructor
	/// @param renderer The audio renderer to produce canvases for
	AudioRendererCanvasCacheFactory(AudioRenderer *renderer);

	/// @brief Create a new canvas
	/// @param i Unused
	/// @return A fresh canvas sized from our master AudioRenderer
	std::unique_ptr<AudioCanvas> ProduceBlock(int i);

	/// @brief Calculate the size of canvases
	/// @return The size of canvases created
	size_t GetBlockSize() const;
};

/// The type of a canvas cache
typedef DataBlockCache<AudioCanvas, 8, AudioRendererCanvasCacheFactory> AudioRendererCanvasCache;


/// @class AudioRenderer
/// @brief Renders audio to offscreen canvases for display on screen
///
/// Manages a canvas cache and paints into the given canvas.
class AudioRenderer {
	friend struct AudioRendererCanvasCacheFactory;

	/// Horizontal zoom level, milliseconds per pixel
	double pixel_ms = 0.f;
	/// Rendering height in pixels
	int pixel_height = 0;
	/// Vertical zoom level/amplitude scale
	float amplitude_scale = 0.f;

	/// Width of canvases to store in cache
	const int cache_bitmap_width = 32; // Completely arbitrary value

	/// Cached canvases for audio ranges
	std::vector<AudioRendererCanvasCache> bitmaps;
	/// The maximum allowed size of each canvas cache, in bytes
	size_t cache_bitmap_maxsize = 0;
	/// The maximum allowed size of the renderer's cache, in bytes
	size_t cache_renderer_maxsize = 0;
	/// Do the caches need to be aged?
	bool needs_age = false;

	/// Actual renderer for canvases
	AudioRendererCanvasProvider *renderer = nullptr;

	/// Audio provider to use as source
	agi::AudioProvider *provider = nullptr;

	/// @brief Make sure canvas index i is in cache
	/// @param i     Index of the canvas to get into cache
	/// @param style Rendering style required for canvas
	/// @return The requested canvas
	AudioCanvas const &GetCachedCanvas(int i, AudioRenderingStyle style);

	/// @brief Update the block count in the canvas caches
	void ResetBlockCount();

	/// Calculate the number of cache blocks needed for a given number of samples
	size_t NumBlocks(int64_t samples) const;

public:
	/// @brief Constructor
	AudioRenderer();

	/// @brief Set horizontal zoom
	/// @param pixel_ms Milliseconds per pixel to render audio at
	void SetMillisecondsPerPixel(double pixel_ms);

	/// @brief Set rendering height
	/// @param pixel_height Height in pixels to render at
	void SetHeight(int pixel_height);

	/// @brief Set vertical zoom
	/// @param amplitude_scale Scaling factor
	void SetAmplitudeScale(float amplitude_scale);

	/// @brief Set the maximum allowed cache size
	/// @param max_size Size in bytes that may be used for caching
	void SetCacheMaxSize(size_t max_size);

	/// @brief Change renderer
	/// @param renderer New renderer to use
	void SetRenderer(AudioRendererCanvasProvider *renderer);

	/// @brief Change audio provider
	/// @param provider New audio provider to use
	void SetAudioProvider(agi::AudioProvider *provider);

	/// @brief Render audio to a canvas
	/// @param canvas Canvas to draw to; its width is the number of pixels to render
	/// @param start  First pixel from beginning of the audio stream to render
	/// @param style  Style to render audio in
	void Render(AudioCanvas &canvas, int start, AudioRenderingStyle style);

	/// @brief Invalidate all cached data
	void Invalidate();
};


/// @class AudioRendererCanvasProvider
/// @brief Base class for audio renderer implementations
class AudioRendererCanvasProvider {
protected:
	/// Audio provider to use for rendering
	agi::AudioProvider *provider;
	/// Horizontal zoom in milliseconds per pixel
	double pixel_ms;
	/// Vertical zoom/amplitude scale factor
	float amplitude_scale;

	/// @brief Called when the audio provider changes
	virtual void OnSetProvider() { }

	/// @brief Called when horizontal zoom changes
	virtual void OnSetMillisecondsPerPixel() { }

	/// @brief Called when vertical zoom changes
	virtual void OnSetAmplitudeScale() { }

public:
	/// @brief Constructor
	AudioRendererCanvasProvider() : provider(nullptr), pixel_ms(0), amplitude_scale(0) { };

	/// @brief Destructor
	virtual ~AudioRendererCanvasProvider() = default;

	/// @brief Rendering function
	/// @param canvas Canvas to render to; its size is the area to fill
	/// @param start  First pixel from beginning of the audio stream to render
	/// @param style  Style to render audio in
	virtual void Render(AudioCanvas &canvas, int start, AudioRenderingStyle style) = 0;

	/// @brief Blank audio rendering function
	/// @param canvas Canvas to render to
	/// @param x      Left edge of the blank range
	/// @param width  Width of the blank range
	/// @param style  Style to render audio in
	virtual void RenderBlank(AudioCanvas &canvas, int x, int width, AudioRenderingStyle style) = 0;

	/// @brief Change audio provider
	void SetProvider(agi::AudioProvider *provider);

	/// @brief Change horizontal zoom
	void SetMillisecondsPerPixel(double new_pixel_ms);

	/// @brief Change vertical zoom
	void SetAmplitudeScale(float amplitude_scale);

	/// @brief Age any caches the renderer might keep
	virtual void AgeCache([[maybe_unused]] size_t max_size) { }
};
