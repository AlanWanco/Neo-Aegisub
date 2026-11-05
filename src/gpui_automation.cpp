// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

#include "gpui_automation.h"

#include "auto4_base.h"
#include "command/command.h"
#include "include/aegisub/context.h"
#include "options.h"

#include <libaegisub/cajun/writer.h>
#include <libaegisub/fs.h>

#include <algorithm>
#include <sstream>
#include <stdexcept>

namespace gpui {
std::string AutomationManager(agi::Context *context, json::Object const& request) {
	std::string const& action = request.at("action");
	auto local = context->local_scripts.get();
	Automation4::ScriptManager *global = config::global_scripts;
	// Load failures and warnings are shown in the native script details below.
	if (action == "add") {
		agi::fs::path path(agi::fs::path(static_cast<std::string const&>(request.at("path"))).lexically_normal().string());
		if (!path.is_absolute() || !agi::fs::FileExists(path))
			throw std::invalid_argument("choose an existing Automation script file");
		for (auto manager : {local, global}) {
			for (auto const& script : manager->GetScripts()) {
				if (script->GetFilename().lexically_normal() == path)
					throw std::invalid_argument("this Automation script is already loaded");
			}
		}
		auto script = Automation4::ScriptFactory::CreateFromFile(path, false, false);
		if (!script) throw std::invalid_argument("no installed Automation engine supports this file");
		local->Add(std::move(script));
		OPT_SET("Path/Last/Automation")->SetString(path.parent_path().string());
	}
	else if (action == "remove" || action == "reload") {
		std::string const& scope = request.at("scope");
		if (scope != "local" && scope != "global") throw std::invalid_argument("unknown Automation scope");
		auto manager = scope == "local" ? local : global;
		std::string const& path = request.at("path");
		auto const& scripts = manager->GetScripts();
		auto found = std::find_if(scripts.begin(), scripts.end(), [&](auto const& script) { return script->GetFilename().string() == path; });
		if (found == scripts.end()) throw std::invalid_argument("Automation script is no longer loaded");
		if (action == "remove") {
			if (scope != "local") throw std::invalid_argument("autoload scripts cannot be removed from a project");
			manager->Remove(found->get());
		}
		else manager->Reload(found->get());
	}
	else if (action == "rescan") global->Reload();
	else if (action == "reload_all") {
		global->Reload();
		// Reload the currently loaded scripts, preserving the project's explicit list.
		// LocalScriptManager::Reload() instead rereads the file and opens a trust prompt.
		std::vector<Automation4::Script *> loaded;
		for (auto const& script : local->GetScripts()) loaded.push_back(script.get());
		for (auto script : loaded) local->Reload(script);
	}
	else if (action != "state") throw std::invalid_argument("unknown Automation manager action");

	json::Array scripts;
	for (auto manager : {local, global}) {
		for (auto const& script : manager->GetScripts()) {
			json::Object entry;
			entry["scope"] = manager == local ? "local" : "global";
			entry["path"] = script->GetFilename().string();
			entry["name"] = script->GetName();
			entry["description"] = script->GetDescription();
			entry["author"] = script->GetAuthor();
			entry["version"] = script->GetVersion();
			entry["loaded"] = script->GetLoadedState();
			json::Array warnings;
			for (auto const& warning : script->GetWarnings()) warnings.emplace_back(warning);
			entry["warnings"] = std::move(warnings);
			json::Array macros;
			for (auto macro : script->GetMacros()) {
				json::Object item;
				item["name"] = macro->name();
				item["label"] = macro->StrDisplay(context);
				item["help"] = macro->StrHelp();
				macros.emplace_back(std::move(item));
			}
			entry["macros"] = std::move(macros);
			json::Array filters;
			for (auto filter : script->GetFilters()) {
				json::Object item;
				item["name"] = filter->GetName();
				item["description"] = filter->GetDescription();
				filters.emplace_back(std::move(item));
			}
			entry["filters"] = std::move(filters);
			scripts.emplace_back(std::move(entry));
		}
	}
	json::Array engines;
	for (auto const& engine : Automation4::ScriptFactory::GetFactories()) {
		json::Object entry;
		entry["name"] = engine->GetEngineName();
		entry["pattern"] = engine->GetFilenamePattern();
		engines.emplace_back(std::move(entry));
	}
	json::Object result;
	result["ok"] = true;
	result["scripts"] = std::move(scripts);
	result["engines"] = std::move(engines);
	std::ostringstream stream;
	agi::JsonWriter::Write(result, stream);
	auto response = stream.str();
	response.erase(std::remove(response.begin(), response.end(), '\n'), response.end());
	return response;
}
}
