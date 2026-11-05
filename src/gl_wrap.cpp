// Copyright (c) 2011, Thomas Goyne <plorkyeran@aegisub.org>
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

/// @file gl_wrap.cpp
/// @brief Geometry for the visual tools, recorded for the front end to draw
/// @ingroup video_output
///

#include "gl_wrap.h"
#include "visual_render.h"

#include <algorithm>

namespace {
constexpr float deg2rad = 3.1415926536f / 180.f;
constexpr float pi = 3.1415926535897932384626433832795f;

// GL primitive numbers. The front end draws these commands itself, so the
// wire format keeps the numbering the capture protocol has always used.
enum Primitive : unsigned {
	PRIMITIVE_POINTS = 0x0000,
	PRIMITIVE_LINES = 0x0001,
	PRIMITIVE_LINE_LOOP = 0x0002,
	PRIMITIVE_LINE_STRIP = 0x0003,
	PRIMITIVE_TRIANGLES = 0x0004,
	PRIMITIVE_TRIANGLE_STRIP = 0x0005,
	PRIMITIVE_TRIANGLE_FAN = 0x0006,
	PRIMITIVE_QUADS = 0x0007,
	PRIMITIVE_QUAD_STRIP = 0x0008,
	PRIMITIVE_POLYGON = 0x0009,
};

/// Vertex storage for one primitive, recorded when it is drawn.
class VertexArray {
	std::vector<float> data;
	size_t dim;
public:
	VertexArray(size_t dims, size_t elems) {
		SetSize(dims, elems);
	}

	void SetSize(size_t dims, size_t elems) {
		dim = dims;
		data.assign(elems * dim, 0.f);
	}

	void Set(size_t i, float x, float y) {
		data[i * dim] = x;
		data[i * dim + 1] = y;
	}

	void Set(size_t i, float x, float y, float z) {
		data[i * dim] = x;
		data[i * dim + 1] = y;
		data[i * dim + 2] = z;
	}

	void Set(size_t i, Vector2D p) {
		data[i * dim] = p.X();
		data[i * dim + 1] = p.Y();
	}

	void Draw(unsigned mode, bool clear = true) {
		if (auto capture = VisualRenderCapture::Current())
			capture->Record(mode, dim, data.data(), data.size() / dim);
		if (clear)
			data.clear();
	}
};
}

void OpenGLWrapper::SetLineColour(agi::Color col, float alpha, int width) {
	line_r = col.r / 255.f;
	line_g = col.g / 255.f;
	line_b = col.b / 255.f;
	line_a = alpha;
	line_width = width;
}

void OpenGLWrapper::SetFillColour(agi::Color col, float alpha) {
	fill_r = col.r / 255.f;
	fill_g = col.g / 255.f;
	fill_b = col.b / 255.f;
	fill_a = alpha;
}

void OpenGLWrapper::SetModeLine() const {
	if (auto capture = VisualRenderCapture::Current())
		capture->Style(line_r, line_g, line_b, line_a, line_width, false);
}

void OpenGLWrapper::SetModeFill() const {
	if (auto capture = VisualRenderCapture::Current())
		capture->Style(fill_r, fill_g, fill_b, fill_a, line_width, true);
}

void OpenGLWrapper::SetInvert() {
}

void OpenGLWrapper::ClearInvert() {
}

void OpenGLWrapper::DrawLine(Vector2D p1, Vector2D p2) const {
	SetModeLine();
	VertexArray buf(2, 2);
	buf.Set(0, p1);
	buf.Set(1, p2);
	buf.Draw(PRIMITIVE_LINES);
}

static inline Vector2D interp(Vector2D p1, Vector2D p2, float t) {
	return t * p1 + (1 - t) * p2;
}

void OpenGLWrapper::DrawDashedLine(Vector2D p1, Vector2D p2, float step) const {
	float const length = (p2 - p1).Len();
	if (length == 0.f) return;
	step /= length;
	for (float t = 0; t < 1.f; t += 2 * step)
		DrawLine(interp(p1, p2, t), interp(p1, p2, t + step));
}

void OpenGLWrapper::DrawEllipse(Vector2D center, Vector2D radius) const {
	DrawRing(center, radius.Y(), radius.Y(), radius.X() / radius.Y());
}

void OpenGLWrapper::DrawRectangle(Vector2D p1, Vector2D p2) const {
	VertexArray buf(2, 4);
	buf.Set(0, p1);
	buf.Set(1, Vector2D(p2, p1));
	buf.Set(2, p2);
	buf.Set(3, Vector2D(p1, p2));

	if (fill_a != 0.f) {
		SetModeFill();
		buf.Draw(PRIMITIVE_QUADS, false);
	}
	if (line_a != 0.f) {
		SetModeLine();
		buf.Draw(PRIMITIVE_LINE_LOOP);
	}
}

void OpenGLWrapper::DrawTriangle(Vector2D p1, Vector2D p2, Vector2D p3) const {
	VertexArray buf(2, 3);
	buf.Set(0, p1);
	buf.Set(1, p2);
	buf.Set(2, p3);

	if (fill_a != 0.f) {
		SetModeFill();
		buf.Draw(PRIMITIVE_TRIANGLES, false);
	}
	if (line_a != 0.f) {
		SetModeLine();
		buf.Draw(PRIMITIVE_LINE_LOOP);
	}
}

void OpenGLWrapper::DrawRing(Vector2D center, float r1, float r2, float ar, float arc_start, float arc_end) const {
	if (r2 > r1)
		std::swap(r1, r2);

	// Arc range
	bool needs_end_caps = arc_start != arc_end;

	arc_end *= deg2rad;
	arc_start *= deg2rad;
	if (arc_end <= arc_start)
		arc_end += 2.f * pi;
	float range = arc_end - arc_start;

	// Math
	int steps = std::max<int>(((r1 + r1 * ar) * range / (2.f * pi)) * 4, 12);
	float step = range / steps;
	float cur_angle = arc_start;

	VertexArray buf(2, steps);

	Vector2D scale_inner = Vector2D(ar, 1) * r1;
	Vector2D scale_outer = Vector2D(ar, 1) * r2;

	if (fill_a != 0.f) {
		SetModeFill();

		// Annulus
		if (r1 != r2) {
			buf.SetSize(2, (steps + 1) * 2);
			for (int i = 0; i <= steps; i++) {
				Vector2D offset = Vector2D::FromAngle(cur_angle);
				buf.Set(i * 2 + 0, center + offset * scale_inner);
				buf.Set(i * 2 + 1, center + offset * scale_outer);
				cur_angle += step;
			}
			buf.Draw(PRIMITIVE_QUAD_STRIP);
		}
		// Circle
		else {
			buf.SetSize(2, steps);
			for (int i = 0; i < steps; i++) {
				buf.Set(i, center + Vector2D::FromAngle(cur_angle) * scale_inner);
				cur_angle += step;
			}
			buf.Draw(PRIMITIVE_POLYGON);
		}

		cur_angle = arc_start;
	}

	if (line_a == 0.f) return;

	// Outer
	steps++;
	buf.SetSize(2, steps);

	SetModeLine();
	for (int i = 0; i < steps; i++) {
		buf.Set(i, center + Vector2D::FromAngle(cur_angle) * scale_outer);
		cur_angle += step;
	}
	buf.Draw(PRIMITIVE_LINE_STRIP);

	// Inner
	if (r1 == r2) return;

	cur_angle = arc_start;
	buf.SetSize(2, steps);
	for (int i = 0; i < steps; i++) {
		buf.Set(i, center + Vector2D::FromAngle(cur_angle) * scale_inner);
		cur_angle += step;
	}
	buf.Draw(PRIMITIVE_LINE_STRIP);

	if (!needs_end_caps) return;

	buf.SetSize(2, 4);
	buf.Set(0, center + Vector2D::FromAngle(arc_start) * scale_inner);
	buf.Set(1, center + Vector2D::FromAngle(arc_start) * scale_outer);
	buf.Set(2, center + Vector2D::FromAngle(arc_end) * scale_inner);
	buf.Set(3, center + Vector2D::FromAngle(arc_end) * scale_outer);
	buf.Draw(PRIMITIVE_LINES);
}

void OpenGLWrapper::DrawLines(size_t dim, std::vector<float> const& lines) {
	DrawLines(dim, lines.data(), lines.size() / dim);
}

void OpenGLWrapper::DrawLines(size_t dim, std::vector<float> const& lines, size_t c_dim, std::vector<float> const& colors) {
	auto capture = VisualRenderCapture::Current();
	if (!capture) return;
	for (size_t i = 0; i + 1 < lines.size() / dim; i += 2) {
		float alpha = c_dim > 3 ? (colors[i * c_dim + 3] + colors[(i + 1) * c_dim + 3]) / 2 : line_a;
		capture->Style(colors[i * c_dim], colors[i * c_dim + 1], colors[i * c_dim + 2], alpha, line_width, false);
		capture->Record(PRIMITIVE_LINES, dim, lines.data() + i * dim, 2);
	}
}

void OpenGLWrapper::DrawLines(size_t dim, const float *lines, size_t n) {
	SetModeLine();
	if (auto capture = VisualRenderCapture::Current())
		capture->Record(PRIMITIVE_LINES, dim, lines, n);
}

void OpenGLWrapper::DrawLineStrip(size_t dim, std::vector<float> const& lines) {
	SetModeLine();
	if (auto capture = VisualRenderCapture::Current())
		capture->Record(PRIMITIVE_LINE_STRIP, dim, lines.data(), lines.size() / dim);
}

void OpenGLWrapper::DrawMultiPolygon(std::vector<float> const& points, std::vector<int> &start, std::vector<int> &count, Vector2D video_pos, Vector2D video_size, bool invert) {
	auto capture = VisualRenderCapture::Current();
	if (!capture) return;

	SetModeFill();
	capture->Mask(points, start, count, video_pos.X(), video_pos.Y(), video_size.X(), video_size.Y(), invert);
	SetModeLine();
	for (size_t i = 0; i < start.size(); ++i)
		capture->Record(PRIMITIVE_LINE_LOOP, 2, points.data() + start[i] * 2, count[i]);
}

void OpenGLWrapper::SetOrigin(Vector2D origin) {
	if (auto capture = VisualRenderCapture::Current())
		capture->Multiply({1,0,0,0, 0,1,0,0, 0,0,1,0, origin.X(),origin.Y(),-1,1});
}

void OpenGLWrapper::SetScale(Vector2D scale) {
	if (auto capture = VisualRenderCapture::Current())
		capture->Multiply({scale.X()/100.,0,0,0, 0,scale.Y()/100.,0,0, 0,0,1,0, 0,0,0,1});
}

void OpenGLWrapper::SetRotation(float x, float y, float z, float zScale) {
	auto capture = VisualRenderCapture::Current();
	if (!capture) return;
	capture->Multiply({2500,0,0,0, 0,2500,0,0, 0,0,1,1, 0,0,2500,2500});
	capture->Multiply({1,0,0,0, 0,1,0,0, 0,0,8./zScale,0, 0,0,0,1});
	double cy = cos(y * deg2rad), sy = sin(y * deg2rad);
	double cx = cos(x * deg2rad), sx = sin(x * deg2rad);
	double cz = cos(z * deg2rad), sz = sin(z * deg2rad);
	capture->Multiply({cy,0,sy,0, 0,1,0,0, -sy,0,cy,0, 0,0,0,1});
	capture->Multiply({1,0,0,0, 0,cx,-sx,0, 0,sx,cx,0, 0,0,0,1});
	capture->Multiply({cz,-sz,0,0, sz,cz,0,0, 0,0,1,0, 0,0,0,1});
}

void OpenGLWrapper::SetShear(float x, float y) {
	if (auto capture = VisualRenderCapture::Current())
		capture->Multiply({1,y,0,0, x,1,0,0, 0,0,1,0, 0,0,0,1});
}

void OpenGLWrapper::ResetTransform() {
	if (auto capture = VisualRenderCapture::Current())
		capture->Reset();
}
