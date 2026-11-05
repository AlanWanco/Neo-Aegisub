// Copyright (c) 2014, Thomas Goyne <plorkyeran@aegisub.org>
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

#include "crash_writer.h"

#include "version.h"

#include <libaegisub/format.h>
#include <libaegisub/fs.h>
#include <libaegisub/util.h>

#include <execinfo.h>

#include <cstdio>
#include <fstream>

using namespace agi;

namespace {
agi::fs::path crashlog_path;

void dump_stack(std::ofstream &fp, std::string const& cause) {
	fp << util::strftime("--- %y-%m-%d %H:%M:%S ------------------\n");
	fp << agi::format("VER - %s\n", GetAegisubLongVersionString());
	fp << agi::format("FTL - Beginning stack dump for \"%s\": \n", cause);

	void *frames[128];
	int count = backtrace(frames, 128);
	char **symbols = backtrace_symbols(frames, count);
	for (int i = 0; i < count; ++i)
		fp << agi::format("%03d - %s\n", i, symbols[i] ? symbols[i] : "?");
	free(symbols);

	fp << "End of stack dump.\n";
	fp << "----------------------------------------\n\n";
}
}

namespace crash_writer {
void Initialize(agi::fs::path const& path) {
	crashlog_path = path / "crashlog.txt";
}

void Cleanup() { }

void Write() {
	std::ofstream file(crashlog_path, std::ios::app);
	if (file.is_open())
		dump_stack(file, "Fatal exception");
}

void Write(std::string const& error) {
	std::ofstream file(crashlog_path, std::ios::app);
	if (file.is_open()) {
		file << util::strftime("--- %y-%m-%d %H:%M:%S ------------------\n");
		file << agi::format("VER - %s\n", GetAegisubLongVersionString());
		file << agi::format("EXC - Neo Aegisub has crashed with unhandled exception \"%s\".\n", error);
		file << "----------------------------------------\n\n";
		file.close();
	}
}
}
