// Copyright (c) 2013, Thomas Goyne <plorkyeran@aegisub.org>
//
// Permission to use, copy, modify, and distribute this software for any
// purpose with or without fee is hereby granted, provided that the above
// copyright notice and this permission notice appear in all copies.
//
// THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
// WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
// MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
// ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
// WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
// ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
// OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
//
// Aegisub Project http://www.aegisub.org/

#pragma once

#include <stddef.h>
#include <vector>

class wxImage;

struct VideoFrame {
	std::vector<unsigned char> data;
	int width;
	int height;
	int pitch;
	bool flipped;
};

wxImage GetImage(VideoFrame const& frame);
wxImage GetImageWithAlpha(VideoFrame const& frame);

// Area-averaging downscale of the frame into a tightly packed width*height*3
// RGB buffer. `dst` must hold at least width*height*3 bytes. Header-only so
// the resampler can be exercised without pulling in wxWidgets.
inline void ShrinkFrameToRgb(VideoFrame const& frame, unsigned char *dst, int target_width, int target_height) {
	unsigned char const *src = frame.data.data();

	// Per-column source ranges, so the pixel loop is a tight accumulator.
	std::vector<int> start_x(target_width);
	std::vector<int> span_x(target_width);
	std::vector<float> inv_x(target_width);
	for (int x = 0; x < target_width; ++x) {
		int const x0 = static_cast<int>(static_cast<long long>(x) * frame.width / target_width);
		int x1 = static_cast<int>(static_cast<long long>(x + 1) * frame.width / target_width);
		if (x1 <= x0)
			x1 = x0 + 1;
		start_x[x] = x0;
		span_x[x] = x1 - x0;
		inv_x[x] = 1.f / static_cast<float>(x1 - x0);
	}

	for (int y = 0; y < target_height; ++y) {
		int const y0 = static_cast<int>(static_cast<long long>(y) * frame.height / target_height);
		int y1 = static_cast<int>(static_cast<long long>(y + 1) * frame.height / target_height);
		if (y1 <= y0)
			y1 = y0 + 1;
		float const inv_y = 1.f / static_cast<float>(y1 - y0);
		unsigned char *out_row = dst + static_cast<size_t>(y) * target_width * 3;

		for (int x = 0; x < target_width; ++x) {
			unsigned acc_b = 0, acc_g = 0, acc_r = 0;
			for (int sy = y0; sy < y1; ++sy) {
				int const row = frame.flipped ? frame.height - 1 - sy : sy;
				unsigned char const *p = src + static_cast<size_t>(row) * frame.pitch + static_cast<size_t>(start_x[x]) * 4;
				for (int sx = 0; sx < span_x[x]; ++sx) {
					acc_b += p[sx * 4 + 0];
					acc_g += p[sx * 4 + 1];
					acc_r += p[sx * 4 + 2];
				}
			}
			float const k = inv_y * inv_x[x];
			out_row[x * 3 + 0] = static_cast<unsigned char>(static_cast<float>(acc_r) * k + 0.5f);
			out_row[x * 3 + 1] = static_cast<unsigned char>(static_cast<float>(acc_g) * k + 0.5f);
			out_row[x * 3 + 2] = static_cast<unsigned char>(static_cast<float>(acc_b) * k + 0.5f);
		}
	}
}

// Area-averaging downscale straight from the frame buffer into a new RGB image.
wxImage GetImageShrunk(VideoFrame const& frame, int width, int height);
