// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

/// @file grid_display_text.h
/// @brief Subtitle grid text with the configured override-tag display mode

#pragma once

#include <string>

class AssDialogue;

/// Text of a line as the subtitle grid shows it: override blocks are kept,
/// replaced with a placeholder, or removed, per Subtitle/Grid/Hide Overrides.
std::string GridDisplayText(AssDialogue const& line);
