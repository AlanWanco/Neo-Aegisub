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

#include "clipboard.h"

#include <sys/wait.h>
#include <unistd.h>

#include <cerrno>
#include <cstring>
#include <string>
#include <vector>

namespace {
std::string run(std::vector<char const *> const& argv, std::string const& input, bool feed) {
	int in_pipe[2] = {-1, -1};
	int out_pipe[2] = {-1, -1};
	if (pipe(in_pipe) || pipe(out_pipe)) return {};

	pid_t child = fork();
	if (child < 0) return {};
	if (child == 0) {
		dup2(feed ? in_pipe[0] : out_pipe[1], feed ? STDIN_FILENO : STDOUT_FILENO);
		close(in_pipe[0]);
		close(in_pipe[1]);
		close(out_pipe[0]);
		close(out_pipe[1]);
		std::vector<char *> args;
		for (auto arg : argv) args.push_back(const_cast<char *>(arg));
		args.push_back(nullptr);
		execvp(args[0], args.data());
		_exit(127);
	}

	close(in_pipe[0]);
	close(out_pipe[1]);
	if (feed) {
		size_t written = 0;
		while (written < input.size()) {
			ssize_t n = write(in_pipe[1], input.data() + written, input.size() - written);
			if (n <= 0) break;
			written += (size_t)n;
		}
		close(in_pipe[1]);
		close(out_pipe[0]);
	}
	else {
		close(in_pipe[1]);
		std::string output;
		char buffer[4096];
		ssize_t n;
		while ((n = read(out_pipe[0], buffer, sizeof buffer)) > 0)
			output.append(buffer, (size_t)n);
		close(out_pipe[0]);
		int status = 0;
		waitpid(child, &status, 0);
		if (!WIFEXITED(status) || WEXITSTATUS(status) != 0) return {};
		return output;
	}

	int status = 0;
	waitpid(child, &status, 0);
	return {};
}

bool have(char const *program) {
	auto path = std::string("/usr/bin/") + program;
	return access(path.c_str(), X_OK) == 0;
}
}

namespace clipboard {
std::string GetText() {
	if (have("wl-paste"))
		return run({"wl-paste", "--no-newline"}, {}, false);
	if (have("xclip"))
		return run({"xclip", "-selection", "clipboard", "-o"}, {}, false);
	return {};
}

void SetText(std::string const& text) {
	if (have("wl-copy")) {
		run({"wl-copy"}, text, true);
		return;
	}
	if (have("xclip"))
		run({"xclip", "-selection", "clipboard"}, text, true);
}
}
