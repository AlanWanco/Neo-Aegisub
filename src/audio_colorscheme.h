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

/// @file audio_colorscheme.h
/// @see audio_colorscheme.cpp
/// @ingroup audio_ui
///
/// Manage colour schemes for the audio display


#pragma once

#include <cstdint>
#include <vector>

#include "utils.h"


/// @class AudioSpectrumColorMap
/// @brief Provides colour maps for audio display rendering
///
/// Maps values from floats in range 0..1 into RGB colour values.
///
/// First create an instance of this class, then call an initialisation function
/// in it to fill the palette with a colour map.
class AudioColorScheme {
	/// The palette data for the map
	std::vector<unsigned char> palette;

	/// Factor to multiply 0..1 values by to map them into the palette range
	size_t factor;

	/// @brief Get a floating point value's colour as a 24-bit RGB pixel
	/// @param val The value to map from
	const unsigned char *get_color(float val) const
	{
		return &palette[mid<size_t>(0, val * factor, factor) * 3];
	}

public:
	/// @brief Constructor
	/// @param prec Bit precision to create the colour map with
	/// @param scheme_name Name of the colour scheme to use
	/// @param audio_rendering_style AudioRenderingStyle to init this colorscheme for
	///
	/// Allocates the palette array to 2^prec entries
	AudioColorScheme(int prec, std::string const& scheme_name, int audio_rendering_style);

	/// @brief Map a floating point value to a BGRA pixel
	/// @param val   [in] The value to map from
	/// @param pixel [out] First byte of the pixel to write
	void map_bgra(float val, unsigned char *pixel) const
	{
		const unsigned char *color = get_color(val);
		pixel[0] = color[2];
		pixel[1] = color[1];
		pixel[2] = color[0];
		pixel[3] = 255;
	}

	/// @brief Get a floating point value's colour as 0xRRGGBB
	/// @param val The value to map from
	uint32_t rgb(float val) const
	{
		const unsigned char *color = get_color(val);
		return (static_cast<uint32_t>(color[0]) << 16) |
		       (static_cast<uint32_t>(color[1]) << 8) |
		        static_cast<uint32_t>(color[2]);
	}
};
