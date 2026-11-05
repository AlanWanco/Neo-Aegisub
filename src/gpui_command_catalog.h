// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

#pragma once

#include <string>
#include <string_view>

namespace agi { struct Context; }

namespace gpui {
class Visual;
/// UI dependency of a legacy command: shared model, wx dialog, or wx control.
std::string_view CommandSurface(std::string_view name);
/// Current-context command catalog and the effective legacy menu hierarchy.
std::string CommandCatalog(agi::Context *context, Visual const& visual);
}
