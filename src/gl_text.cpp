// Copyright (c) 2007, Rodrigo Braz Monteiro
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

/// @file gl_text.cpp
/// @brief Text for the visual tools, recorded for the front end to draw
/// @ingroup video_output
///

#include "gl_text.h"
#include "visual_render.h"

#include <algorithm>
#include <cmath>

void OpenGLText::SetFont(std::string const& face, int size, bool bold, bool italics) {
	fontFace = face;
	fontSize = size;
	fontBold = bold;
	fontItalics = italics;
}

void OpenGLText::SetColour(agi::Color col) {
	r = col.r / 255.f;
	g = col.g / 255.f;
	b = col.b / 255.f;
	a = col.a / 255.f;
}

void OpenGLText::Print(std::string const& text, int x, int y) {
	if (auto capture = VisualRenderCapture::Current())
		capture->Text(text, x, y, fontSize, r, g, b, a);
}

void OpenGLText::GetExtent(std::string const& text, int &w, int &h) {
	// The front end shapes the label; this only places it, so an average glyph
	// width is enough to choose which side of the cursor the text goes on.
	w = static_cast<int>(std::lround(text.size() * fontSize * .62));
	h = fontSize;
}
