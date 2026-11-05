// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

/// @file audio_canvas.h
/// @ingroup audio_ui
/// @brief Offscreen BGRA raster the audio renderers draw into

#pragma once

#include <algorithm>
#include <cstdint>
#include <vector>

/// @brief A plain BGRA8 raster with rows stored top-down
///
/// The audio renderers used to draw through wxDC into wxBitmap. The front end
/// only ever needs the pixels, so they now paint into this buffer and the bytes
/// travel to the GPUI window unchanged.
class AudioCanvas {
	int width_ = 0;
	int height_ = 0;
	std::vector<unsigned char> pixels_;

public:
	void Reset(int width, int height) {
		width_ = std::max(0, width);
		height_ = std::max(0, height);
		pixels_.assign(static_cast<size_t>(width_) * static_cast<size_t>(height_) * 4, 0);
	}

	int Width() const { return width_; }
	int Height() const { return height_; }
	unsigned char *Data() { return pixels_.data(); }
	unsigned char const *Data() const { return pixels_.data(); }
	size_t Size() const { return pixels_.size(); }

	/// Fill the whole raster with one opaque colour, given as 0xRRGGBB
	void Clear(uint32_t rgb) {
		FillRect(0, 0, width_, height_, rgb);
	}

	/// Fill an opaque 0xRRGGBB rectangle, clipped to the raster
	void FillRect(int x, int y, int w, int h, uint32_t rgb) {
		int const x0 = std::max(0, x);
		int const y0 = std::max(0, y);
		int const x1 = std::min(width_, x + w);
		int const y1 = std::min(height_, y + h);
		if (x0 >= x1 || y0 >= y1) return;
		unsigned char const r = static_cast<unsigned char>(rgb >> 16);
		unsigned char const g = static_cast<unsigned char>(rgb >> 8);
		unsigned char const b = static_cast<unsigned char>(rgb);
		for (int row = y0; row < y1; ++row) {
			unsigned char *px = pixels_.data() + (static_cast<size_t>(row) * width_ + x0) * 4;
			for (int col = x0; col < x1; ++col, px += 4) {
				px[0] = b;
				px[1] = g;
				px[2] = r;
				px[3] = 255;
			}
		}
	}

	/// Draw a vertical run including both endpoints, clipped to the raster
	void VerticalLine(int x, int y0, int y1, uint32_t rgb) {
		if (y1 < y0) std::swap(y0, y1);
		FillRect(x, y0, 1, y1 - y0 + 1, rgb);
	}

	/// Draw a horizontal run including both endpoints, clipped to the raster
	void HorizontalLine(int y, int x0, int x1, uint32_t rgb) {
		if (x1 < x0) std::swap(x0, x1);
		FillRect(x0, y, x1 - x0 + 1, 1, rgb);
	}

	/// Copy another raster in at (dx, dy), clipping anything outside this one
	void Blit(AudioCanvas const &source, int dx, int dy) {
		int const sx0 = std::max(0, -dx);
		int const sy0 = std::max(0, -dy);
		int const sx1 = std::min(source.width_, width_ - dx);
		int const sy1 = std::min(source.height_, height_ - dy);
		if (sx0 >= sx1 || sy0 >= sy1) return;
		size_t const bytes = static_cast<size_t>(sx1 - sx0) * 4;
		for (int y = sy0; y < sy1; ++y) {
			unsigned char const *src = source.pixels_.data() + (static_cast<size_t>(y) * source.width_ + sx0) * 4;
			unsigned char *dst = pixels_.data() + (static_cast<size_t>(y + dy) * width_ + (sx0 + dx)) * 4;
			std::copy(src, src + bytes, dst);
		}
	}
};
