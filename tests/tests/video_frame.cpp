// Copyright (c) 2026, Aegisub Project
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

/// @file video_frame.cpp
/// @brief Test suite for frame conversion helpers

#include "../../src/video_frame.h"

#include <gtest/gtest.h>

#include <vector>

namespace {
// Decoder buffers are BGRA, with the fourth byte unused.
VideoFrame make_frame(int width, int height, unsigned char b, unsigned char g, unsigned char r) {
	VideoFrame frame;
	frame.width = width;
	frame.height = height;
	frame.pitch = width * 4;
	frame.flipped = false;
	frame.data.assign(static_cast<size_t>(width) * height * 4, 0);
	for (size_t i = 0; i < frame.data.size(); i += 4) {
		frame.data[i + 0] = b;
		frame.data[i + 1] = g;
		frame.data[i + 2] = r;
	}
	return frame;
}

std::vector<unsigned char> shrink(VideoFrame const& frame, int width, int height) {
	std::vector<unsigned char> out(static_cast<size_t>(width) * height * 3, 0xCD);
	ShrinkFrameToRgb(frame, out.data(), width, height);
	return out;
}

void expect_rgb(std::vector<unsigned char> const& image, int width, int x, int y,
                unsigned char r, unsigned char g, unsigned char b) {
	size_t const offset = (static_cast<size_t>(y) * width + x) * 3;
	EXPECT_EQ(r, image[offset + 0]) << "red at " << x << "," << y;
	EXPECT_EQ(g, image[offset + 1]) << "green at " << x << "," << y;
	EXPECT_EQ(b, image[offset + 2]) << "blue at " << x << "," << y;
}
}

TEST(VideoFrameShrink, KeepsChannelOrder) {
	VideoFrame frame = make_frame(4, 4, 30, 60, 90);
	auto image = shrink(frame, 2, 2);

	for (int y = 0; y < 2; ++y)
		for (int x = 0; x < 2; ++x)
			expect_rgb(image, 2, x, y, 90, 60, 30);
}

TEST(VideoFrameShrink, AveragesEachSourceBlock) {
	// Left half blue, right half red: each output pixel takes the mean of its
	// 2x2 source block rather than a single sample.
	VideoFrame frame = make_frame(4, 4, 0, 0, 0);
	for (int y = 0; y < 4; ++y) {
		for (int x = 0; x < 4; ++x) {
			unsigned char *p = frame.data.data() + (static_cast<size_t>(y) * 4 + x) * 4;
			p[0] = x < 2 ? 200 : 0;
			p[2] = x < 2 ? 0 : 100;
		}
	}

	auto image = shrink(frame, 2, 2);

	expect_rgb(image, 2, 0, 0, 0, 0, 200);
	expect_rgb(image, 2, 1, 0, 100, 0, 0);
}

TEST(VideoFrameShrink, HonoursFlippedFrames) {
	// Blue carries the row index, so a vertical flip is visible.
	VideoFrame frame = make_frame(2, 4, 0, 0, 0);
	for (int y = 0; y < 4; ++y)
		for (int x = 0; x < 2; ++x)
			frame.data[(static_cast<size_t>(y) * 2 + x) * 4] = static_cast<unsigned char>(y * 10);

	auto upright = shrink(frame, 2, 2);
	expect_rgb(upright, 2, 0, 0, 0, 0, 5);
	expect_rgb(upright, 2, 0, 1, 0, 0, 25);

	frame.flipped = true;
	auto flipped = shrink(frame, 2, 2);
	expect_rgb(flipped, 2, 0, 0, 0, 0, 25);
	expect_rgb(flipped, 2, 0, 1, 0, 0, 5);
}

TEST(VideoFrameShrink, HandlesNonIntegerRatios) {
	VideoFrame frame = make_frame(6, 6, 40, 80, 120);
	auto image = shrink(frame, 4, 4);

	for (int y = 0; y < 4; ++y)
		for (int x = 0; x < 4; ++x)
			expect_rgb(image, 4, x, y, 120, 80, 40);
}
