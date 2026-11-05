// Copyright (c) 2026, Aegisub contributors
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

#include "image_png.h"

#include <zlib.h>

#include <cstring>
#include <fstream>
#include <stdexcept>
#include <string>

namespace {
void put32(std::vector<unsigned char> &out, uint32_t value) {
	out.push_back(static_cast<unsigned char>(value >> 24));
	out.push_back(static_cast<unsigned char>(value >> 16));
	out.push_back(static_cast<unsigned char>(value >> 8));
	out.push_back(static_cast<unsigned char>(value));
}

void chunk(std::vector<unsigned char> &out, char const *type, std::vector<unsigned char> const &payload) {
	put32(out, static_cast<uint32_t>(payload.size()));
	size_t start = out.size();
	out.insert(out.end(), type, type + 4);
	out.insert(out.end(), payload.begin(), payload.end());
	uint32_t crc = crc32(0, out.data() + start, static_cast<uInt>(4 + payload.size()));
	put32(out, crc);
}

std::vector<unsigned char> deflate(unsigned char const *data, size_t size) {
	uLongf bound = compressBound(static_cast<uLong>(size));
	std::vector<unsigned char> out(bound);
	if (compress2(out.data(), &bound, data, static_cast<uLong>(size), 6) != Z_OK)
		throw std::runtime_error("PNG compression failed");
	out.resize(bound);
	return out;
}
}

std::vector<unsigned char> EncodePng(int width, int height, unsigned char const *rgba, bool alpha) {
	if (width <= 0 || height <= 0 || !rgba)
		throw std::runtime_error("invalid PNG dimensions");

	size_t const channels = alpha ? 4 : 3;
	size_t const row_bytes = static_cast<size_t>(width) * channels;
	std::vector<unsigned char> raw((row_bytes + 1) * static_cast<size_t>(height));
	for (int y = 0; y < height; ++y) {
		unsigned char *row = raw.data() + static_cast<size_t>(y) * (row_bytes + 1);
		row[0] = 0;
		if (alpha)
			memcpy(row + 1, rgba + static_cast<size_t>(y) * row_bytes, row_bytes);
		else {
			unsigned char const *src = rgba + static_cast<size_t>(y) * row_bytes;
			for (int x = 0; x < width; ++x) {
				row[1 + x * 3 + 0] = src[x * 3 + 0];
				row[1 + x * 3 + 1] = src[x * 3 + 1];
				row[1 + x * 3 + 2] = src[x * 3 + 2];
			}
		}
	}

	std::vector<unsigned char> out = {0x89, 'P', 'N', 'G', '\r', '\n', 0x1a, '\n'};
	std::vector<unsigned char> ihdr;
	put32(ihdr, static_cast<uint32_t>(width));
	put32(ihdr, static_cast<uint32_t>(height));
	ihdr.push_back(8);
	ihdr.push_back(alpha ? 6 : 2);
	ihdr.push_back(0);
	ihdr.push_back(0);
	ihdr.push_back(0);
	chunk(out, "IHDR", ihdr);
	chunk(out, "IDAT", deflate(raw.data(), raw.size()));
	chunk(out, "IEND", {});
	return out;
}

void WritePng(agi::fs::path const& filename, int width, int height, unsigned char const *rgba, bool alpha) {
	auto bytes = EncodePng(width, height, rgba, alpha);
	std::ofstream file(filename, std::ios::binary);
	if (!file) throw std::runtime_error("could not open " + filename.string());
	file.write(reinterpret_cast<char const *>(bytes.data()), static_cast<std::streamsize>(bytes.size()));
	if (!file) throw std::runtime_error("could not write " + filename.string());
}
