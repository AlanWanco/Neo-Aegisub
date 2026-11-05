// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

#pragma once

#include "vector2d.h"

/// What the pointer did, in the vocabulary the visual tools act on.
enum class VisualMouseAction {
	Move,
	Down,
	Up,
	DoubleClick,
	Leave
};

/// A pointer event from whichever window is hosting the tools.
struct VisualMouseEvent {
	VisualMouseAction action = VisualMouseAction::Move;
	int x = 0;
	int y = 0;
	/// Is the left button held after this event?
	bool left = false;
	bool shift = false;
	bool ctrl = false;
	bool alt = false;

	bool LeftDown() const { return action == VisualMouseAction::Down || action == VisualMouseAction::DoubleClick; }
	bool LeftDClick() const { return action == VisualMouseAction::DoubleClick; }
	bool LeftIsDown() const { return left; }
	bool Leaving() const { return action == VisualMouseAction::Leave; }
	bool ShiftDown() const { return shift; }
	bool CmdDown() const { return ctrl; }
	bool AltDown() const { return alt; }
	Vector2D Position() const { return Vector2D(x, y); }
};

/// Window interaction needed by the shared visual typesetting tools.
class VisualToolHost {
public:
	virtual ~VisualToolHost() = default;
	virtual void Render() = 0;
	virtual bool HasCapture() const = 0;
	virtual void CaptureMouse() = 0;
	virtual void ReleaseMouse() = 0;
	virtual void SetFocus() = 0;
	virtual void SetCrosshairCursor(bool hidden) = 0;
};
