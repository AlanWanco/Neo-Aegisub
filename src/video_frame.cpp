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

#include "video_frame.h"

RasterImage GetRgbImage(VideoFrame const& frame) {
	RasterImage out;
	out.width = frame.width;
	out.height = frame.height;
	out.channels = 3;
	out.data.resize(static_cast<size_t>(frame.width) * frame.height * 3);
	for (int y = 0; y < frame.height; ++y) {
		int const row = frame.flipped ? frame.height - 1 - y : y;
		unsigned char const *src = frame.data.data() + static_cast<size_t>(row) * frame.pitch;
		unsigned char *dst = out.data.data() + static_cast<size_t>(y) * frame.width * 3;
		for (int x = 0; x < frame.width; ++x) {
			dst[x * 3 + 0] = src[x * 4 + 2];
			dst[x * 3 + 1] = src[x * 4 + 1];
			dst[x * 3 + 2] = src[x * 4 + 0];
		}
	}
	return out;
}

RasterImage GetRgbaImage(VideoFrame const& frame) {
	RasterImage out;
	out.width = frame.width;
	out.height = frame.height;
	out.channels = 4;
	out.data.resize(static_cast<size_t>(frame.width) * frame.height * 4);
	for (int y = 0; y < frame.height; ++y) {
		int const row = frame.flipped ? frame.height - 1 - y : y;
		unsigned char const *src = frame.data.data() + static_cast<size_t>(row) * frame.pitch;
		unsigned char *dst = out.data.data() + static_cast<size_t>(y) * frame.width * 4;
		for (int x = 0; x < frame.width; ++x) {
			dst[x * 4 + 0] = src[x * 4 + 2];
			dst[x * 4 + 1] = src[x * 4 + 1];
			dst[x * 4 + 2] = src[x * 4 + 0];
			dst[x * 4 + 3] = src[x * 4 + 3];
		}
	}
	return out;
}
