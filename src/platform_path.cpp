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
//
// Aegisub Project http://www.aegisub.org/

#include "platform_path.h"

#if defined(__APPLE__)
#include <mach-o/dyld.h>
#elif defined(_WIN32)
#include <windows.h>
#else
#include <unistd.h>
#endif

namespace agi::util {
std::filesystem::path ExecutablePath(char const *argv0) {
#if defined(__APPLE__)
	char buffer[4096];
	uint32_t size = sizeof buffer;
	if (_NSGetExecutablePath(buffer, &size) == 0) {
		std::error_code error;
		auto resolved = std::filesystem::weakly_canonical(buffer, error);
		if (!error) return resolved;
	}
#elif defined(_WIN32)
	wchar_t buffer[MAX_PATH];
	if (GetModuleFileNameW(nullptr, buffer, MAX_PATH) > 0)
		return std::filesystem::path(buffer);
#else
	std::error_code error;
	auto resolved = std::filesystem::read_symlink("/proc/self/exe", error);
	if (!error) return resolved;
#endif
	if (argv0 && *argv0) {
		std::error_code error;
		auto resolved = std::filesystem::weakly_canonical(argv0, error);
		if (!error) return resolved;
	}
	return {};
}
}
