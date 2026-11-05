// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

/// @file
/// @brief Headless host process for the GPUI frontend.
///
/// The host owns the document, the bridge socket and the video/audio decode
/// pipeline. It draws nothing: every surface the user sees is a GPUI window in
/// the frontend process, which talks to this process over a loopback socket.
/// This file is the whole of the host's startup: build the configuration, open
/// the document, start the socket, launch the frontend and pump the loop.

#include "aegisublocale.h"
#include "auto4_base.h"
#include "auto4_lua_factory.h"
#include "compat.h"
#include "command/command.h"
#include "crash_writer.h"
#include "export_fixstyle.h"
#include "export_framerate.h"
#include "format.h"
#include "gpui_bridge.h"
#include "gpui_loop.h"
#include "include/aegisub/context.h"
#include "include/aegisub/hotkey.h"
#include "libresrc/libresrc.h"
#include "options.h"
#include "project.h"
#include "subtitles_provider_libass.h"
#include "utils.h"
#include "version.h"

#include <libaegisub/dispatch.h>
#include <libaegisub/exception.h>
#include <libaegisub/fs.h>
#include <libaegisub/log.h>
#include <libaegisub/mru.h>
#include <libaegisub/option.h>
#include <libaegisub/path.h>
#include <libaegisub/util.h>

#include <wx/image.h>
#include <wx/init.h>

#include <chrono>
#include <csignal>
#include <cstdio>
#include <cstdlib>
#include <filesystem>
#include <memory>
#include <string>
#include <vector>

#if defined(__APPLE__)
#include <mach-o/dyld.h>
#include <spawn.h>
#include <sys/wait.h>
#include <unistd.h>
extern char **environ;
#elif defined(_WIN32)
#include <process.h>
#include <windows.h>
#else
#include <spawn.h>
#include <sys/wait.h>
#include <unistd.h>
extern char **environ;
#endif

namespace config {
agi::Path *path = nullptr;
agi::Options *opt = nullptr;
agi::MRUManager *mru = nullptr;
Automation4::AutoloadScriptManager *global_scripts = nullptr;
}

AegisubLocale ui_locale;

namespace {
using namespace std::chrono_literals;

void StartupLog(std::string const& message) {
	LOG_I("startup") << message;
}

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

/// The frontend sits next to the host in an installed bundle and under
/// gpui-prototype/target/release in a checkout.
std::filesystem::path FrontendPath(char const *argv0) {
	std::string const name =
#if defined(_WIN32)
		"aegisub-gpui-prototype.exe";
#else
		"aegisub-gpui-prototype";
#endif
	auto const executable = ExecutablePath(argv0);
	if (!executable.empty()) {
		auto sibling = executable.parent_path() / name;
		if (std::filesystem::exists(sibling)) return sibling;
	}

	std::vector<std::filesystem::path> roots;
	std::error_code error;
	roots.push_back(std::filesystem::current_path(error));
	if (!executable.empty()) {
		auto directory = executable.parent_path();
		roots.push_back(directory);
#if defined(__APPLE__)
		// aegisub.app/Contents/MacOS/aegisub -> the checkout above the bundle
		if (directory.filename() == "MacOS" && directory.parent_path().filename() == "Contents")
			roots.push_back(directory.parent_path().parent_path().parent_path().parent_path());
#endif
	}
	for (auto const& root : roots) {
		auto candidate = root / "gpui-prototype" / "target" / "release" / name;
		if (std::filesystem::exists(candidate)) return candidate;
	}
	return {};
}

/// A child process the host watches: when the frontend exits there is no UI
/// left, so the host shuts down.
class ChildProcess {
public:
	ChildProcess() = default;
	~ChildProcess() { Kill(); }
	ChildProcess(ChildProcess const&) = delete;
	ChildProcess& operator=(ChildProcess const&) = delete;

	bool Spawn(std::filesystem::path const& program, std::vector<std::string> const& arguments) {
#if defined(_WIN32)
		std::string command_line;
		for (auto const& argument : arguments) {
			if (!command_line.empty()) command_line += ' ';
			command_line += '"';
			command_line += argument;
			command_line += '"';
		}
		STARTUPINFOA startup {};
		startup.cb = sizeof startup;
		PROCESS_INFORMATION info {};
		std::string application = program.string();
		if (!CreateProcessA(application.c_str(), command_line.data(), nullptr, nullptr, FALSE,
			0, nullptr, nullptr, &startup, &info))
			return false;
		process = info.hProcess;
		CloseHandle(info.hThread);
		return true;
#else
		std::vector<std::string> storage;
		storage.reserve(arguments.size() + 1);
		storage.push_back(program.string());
		storage.insert(storage.end(), arguments.begin(), arguments.end());
		std::vector<char *> argv;
		argv.reserve(storage.size() + 1);
		for (auto& argument : storage) argv.push_back(argument.data());
		argv.push_back(nullptr);

		pid_t child = 0;
		posix_spawn_file_actions_t actions;
		posix_spawn_file_actions_init(&actions);
		int const result = posix_spawn(&child, program.c_str(), &actions, nullptr, argv.data(), environ);
		posix_spawn_file_actions_destroy(&actions);
		if (result != 0) return false;
		pid = child;
		return true;
#endif
	}

	bool Running() {
#if defined(_WIN32)
		return process && WaitForSingleObject(process, 0) == WAIT_TIMEOUT;
#else
		if (!pid) return false;
		int status = 0;
		pid_t const result = waitpid(pid, &status, WNOHANG);
		if (result == 0) return true;
		pid = 0;
		return false;
#endif
	}

	void Kill() {
#if defined(_WIN32)
		if (process) {
			TerminateProcess(process, 0);
			CloseHandle(process);
			process = nullptr;
		}
#else
		if (pid) {
			kill(pid, SIGTERM);
			int status = 0;
			waitpid(pid, &status, 0);
			pid = 0;
		}
#endif
	}

private:
#if defined(_WIN32)
	void *process = nullptr;
#else
	pid_t pid = 0;
#endif
};

int Run(char const *argv0, std::vector<std::string> const& files) {
	// Logging has to exist before anything can log: every Path, Options and
	// dispatch step reports through it.
	agi::log::log = new agi::log::LogSink;
#ifdef _DEBUG
	agi::log::log->Subscribe(std::make_unique<agi::log::EmitSTDOUT>());
#endif

	StartupLog("Initialise locale");
	agi::util::InitLocale();

	StartupLog("Initialise dispatch");
	agi::dispatch::Init([](agi::dispatch::Thunk thunk) {
		gpui::EventLoop::Current().Post(std::move(thunk));
	});

	config::path = new agi::Path;

	StartupLog("Create log writer");
	auto log_directory = config::path->Decode("?user/log/");
	agi::fs::CreateDirectory(log_directory);
	agi::log::log->Subscribe(std::make_unique<agi::log::JsonEmitter>(log_directory));
	CleanCache(log_directory, "*.json", 10, 1000);

	StartupLog("Load user configuration");
	config::opt = new agi::Options(config::path->Decode("?user/config.json"), GET_DEFAULT_CONFIG(default_config));
	try {
		config::opt->ConfigUser();
	}
	catch (agi::Exception const& err) {
		LOG_E("config/init") << "Caught exception: " << err.GetMessage();
	}

	crash_writer::Initialize(config::path->Decode("?user"));

	StartupLog("Init commands");
	cmd::init_builtin_commands();
	hotkey::init();

	StartupLog("Load MRU");
	config::mru = new agi::MRUManager(config::path->Decode("?user/mru.json"),
		GET_DEFAULT_CONFIG(default_mru), config::opt);

	StartupLog("Store options back");
	OPT_SET("Version/Last Version")->SetInt(GetSVNRevision());

	StartupLog("Initialize final locale");
	auto language = OPT_GET("App/Language")->GetString();
#ifdef WITH_GPUI_DEFAULT
	// The GPUI workspace only ships Chinese strings, so it runs in Chinese
	// unless the stored language actually has a catalog.
	if (language.empty() || !ui_locale.HasLanguage(language))
		language = "zh_CN";
#else
	if (language.empty() || (language != "en_US" && !ui_locale.HasLanguage(language)))
		language = ui_locale.RecommendLanguage();
#endif
	ui_locale.Init(language);

	StartupLog("Install image handlers");
	wxImage::AddHandler(new wxPNGHandler);
#if wxUSE_LIBJPEG
	wxImage::AddHandler(new wxJPEGHandler);
#endif

	StartupLog("Load plugins");
	Automation4::ScriptFactory::Register(std::make_unique<Automation4::LuaScriptFactory>());
	libass::CacheFonts();

	StartupLog("Register export filters");
	AssExportFilterChain::Register(std::make_unique<AssFixStylesFilter>());
	AssExportFilterChain::Register(std::make_unique<AssTransformFramerateFilter>());

	auto context = std::make_unique<agi::Context>();

	StartupLog("Open command line files");
	if (!files.empty()) {
		std::vector<agi::fs::path> paths;
		for (auto const& file : files) paths.emplace_back(file);
		context->project->LoadList(paths);
	}

	StartupLog("Start bridge");
	GpuiBridge bridge(context.get());
	if (!bridge.Start()) {
		std::fprintf(stderr, "gpui-host: could not start the bridge\n");
		return 1;
	}

	auto frontend = FrontendPath(argv0);
	if (frontend.empty()) {
		std::fprintf(stderr, "gpui-host: aegisub-gpui-prototype not found\n");
		return 1;
	}

	auto const endpoint = bridge.Endpoint();
	auto const token = bridge.Token();
	ChildProcess child;
	if (!child.Spawn(frontend, {"--bridge", endpoint, "--token", token})) {
		std::fprintf(stderr, "gpui-host: could not launch %s\n", frontend.string().c_str());
		return 1;
	}

	auto& loop = gpui::EventLoop::Current();
	loop.Attach(&bridge.Server());

	gpui::Timer watch_frontend;
	watch_frontend.on_tick = [&] {
		if (!child.Running()) loop.Stop();
	};
	watch_frontend.Start(250);

	StartupLog("Initialization complete");
	loop.Run();

	watch_frontend.Stop();
	child.Kill();
	bridge.Stop();

	// The context owns the document, the controllers and the bridge's client
	// state, so it has to go before the configuration it reads from.
	context.reset();

	delete config::global_scripts;
	config::global_scripts = nullptr;
	AssExportFilterChain::Clear();
	hotkey::clear();
	cmd::clear();
	delete config::mru;
	delete config::opt;
	crash_writer::Cleanup();
	// Keep this last: background tasks post their results through the log.
	delete agi::log::log;
	agi::log::log = nullptr;
	config::mru = nullptr;
	config::opt = nullptr;
	delete config::path;
	config::path = nullptr;
	return 0;
}
}

int main(int argc, char **argv) {
	wxInitializer initializer;
	if (!initializer.IsOk()) {
		std::fprintf(stderr, "gpui-host: wxWidgets failed to initialise\n");
		return 1;
	}

	std::vector<std::string> files;
	for (int i = 1; i < argc; ++i)
		files.emplace_back(argv[i]);

	try {
		return Run(argv[0], files);
	}
	catch (agi::Exception const& err) {
		std::fprintf(stderr, "gpui-host: %s\n", err.GetMessage().c_str());
	}
	catch (std::exception const& err) {
		std::fprintf(stderr, "gpui-host: %s\n", err.what());
	}
	catch (...) {
		std::fprintf(stderr, "gpui-host: unknown error during startup\n");
	}
	return 1;
}
