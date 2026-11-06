#pragma once

#include <string_view>

namespace gpui {
/// Append one line to the host trace file next to the executable. No-op on
/// platforms other than Windows; used to diagnose early host exits.
void TraceHost(std::string_view message);
}
