// Copyright (c) 2007, Rodrigo Braz Monteiro
// All rights reserved.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are met:
//
//   * Redistributions of source code must retain the above copyright notice,
//     this list of conditions and the following disclaimer.
//   * Redistributions in binary form must reproduce the above copyright notice,
//     this list of conditions and the following disclaimer in the documentation
//     and/or other materials provided with the distribution.
//   * Neither the name of the Aegisub Group nor the names of its contributors
//     may be used to endorse or promote products derived from this software
//     without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
// AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
// IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
// ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE
// LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
// CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
// SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
// INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
// CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
// ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
// POSSIBILITY OF SUCH DAMAGE.
//
// Aegisub Project http://www.aegisub.org/

/// @file version_check.cpp
/// @brief Check GitHub releases for a newer build
/// @ingroup help_menu

#include "dialogs.h"

#include "release_manifest.h"
#include "version.h"

#include <mutex>

#ifdef WITH_UPDATE_CHECKER
#include "gpui_bridge.h"
#include "options.h"

#include <libaegisub/cajun/reader.h>
#include <libaegisub/dispatch.h>
#include <libaegisub/format.h>

#include <ctime>
#include <curl/curl.h>
#include <memory>
#include <sstream>
#include <stdexcept>

namespace {
std::mutex VersionCheckLock;
bool checking = false;
bool interactive_check = false;
std::string check_status = "idle";
std::string check_message;
std::optional<updates::Release> check_release;

size_t WriteResponse(char *contents, size_t size, size_t count, void *target) {
	try {
		static_cast<std::string *>(target)->append(contents, size * count);
		return size * count;
	}
	catch (...) { return 0; }
}

std::optional<updates::Release> DoCheck() {
	std::unique_ptr<CURL, decltype(&curl_easy_cleanup)> curl(curl_easy_init(), curl_easy_cleanup);
	if (!curl) throw std::runtime_error("Curl could not be initialized.");
	std::unique_ptr<curl_slist, decltype(&curl_slist_free_all)> headers(
		curl_slist_append(nullptr, "Accept: application/vnd.github+json"), curl_slist_free_all);
	if (!headers) throw std::runtime_error("Could not allocate GitHub request headers.");
	curl_easy_setopt(curl.get(), CURLOPT_URL, "https://api.github.com/repos/AlanWanco/Neo-Aegisub/releases?per_page=100");
	curl_easy_setopt(curl.get(), CURLOPT_HTTPHEADER, headers.get());
	curl_easy_setopt(curl.get(), CURLOPT_USERAGENT, "Neo Aegisub-update-checker/1.0");
	curl_easy_setopt(curl.get(), CURLOPT_CONNECTTIMEOUT, 10L);
	curl_easy_setopt(curl.get(), CURLOPT_TIMEOUT, 30L);
	curl_easy_setopt(curl.get(), CURLOPT_NOSIGNAL, 1L);
	// GitHub answers the pre-rename repository path with a 301 to the renamed
	// one; builds shipped before the rename still hard-code that old path.
	curl_easy_setopt(curl.get(), CURLOPT_FOLLOWLOCATION, 1L);
	std::string response;
	curl_easy_setopt(curl.get(), CURLOPT_WRITEFUNCTION, WriteResponse);
	curl_easy_setopt(curl.get(), CURLOPT_WRITEDATA, &response);
	auto result = curl_easy_perform(curl.get());
	if (result != CURLE_OK)
		throw std::runtime_error(std::string("Checking for updates failed: ") + curl_easy_strerror(result));
	long status = 0;
	curl_easy_getinfo(curl.get(), CURLINFO_RESPONSE_CODE, &status);
	if (status < 200 || status >= 300)
		throw std::runtime_error("GitHub releases request failed (HTTP " + std::to_string(status) + ").");
	std::istringstream stream(response);
	json::UnknownElement manifest;
	json::Reader::Read(manifest, stream);
	return updates::FindUpdate(static_cast<json::Array const&>(manifest), GetSVNRevision(), GetVersionNumbers());
}
}

json::Object GetVersionCheckState() {
	std::lock_guard lock(VersionCheckLock);
	json::Object result;
	result["enabled"] = true;
	result["checking"] = checking;
	result["status"] = check_status;
	result["current_version"] = std::string(GetAegisubLongVersionString());
	result["source"] = updates::releases_url;
	result["message"] = check_message;
	json::Array releases;
	if (check_release) {
		json::Object release;
		release["name"] = check_release->name;
		release["url"] = check_release->url;
		release["description"] = check_release->description;
		releases.emplace_back(std::move(release));
	}
	result["releases"] = std::move(releases);
	return result;
}

void PerformVersionCheck(bool interactive) {
	if (!interactive && (!OPT_GET("App/Auto/Check For Updates")->GetBool() ||
	    OPT_GET("Version/Next Check")->GetInt() > time(nullptr))) return;
	{
		std::lock_guard lock(VersionCheckLock);
		interactive_check = interactive_check || interactive;
		if (checking) return;
		checking = true;
		check_status = "checking";
		check_message = "Checking GitHub releases…";
		check_release.reset();
	}
	agi::dispatch::Background().Async([] {
		std::optional<updates::Release> release;
		std::string status;
		std::string message;
		try {
			release = DoCheck();
			status = release ? "available" : "current";
			message = release ? "An update to Neo Aegisub was found." : "There are no updates to Neo Aegisub.";
		}
		catch (std::exception const& error) {
			status = "error";
			message = "There was an error checking for updates to Neo Aegisub: " + std::string(error.what());
		}
		catch (...) {
			status = "error";
			message = "An unknown error occurred while checking for updates to Neo Aegisub.";
		}
		std::string title;
		std::string body;
		bool show;
		{
			std::lock_guard lock(VersionCheckLock);
			check_status = status;
			check_message = message;
			check_release = release;
			checking = false;
			show = interactive_check || static_cast<bool>(release);
			interactive_check = false;
			if (release) {
				title = "发现新版本 " + release->name;
				body = release->description + "\n\n" + release->url;
			}
			else {
				title = "版本检查";
				body = message;
			}
		}
		agi::dispatch::Main().Async([show, title, body] {
			OPT_SET("Version/Next Check")->SetInt(time(nullptr) + 60*60);
			// Interactive checks report through the front end's version page; an
			// automatic one only speaks up when there is something to install.
			if (show) gpui::PushNotice(title, body);
		});
	});
}

#else
void PerformVersionCheck(bool) { }

json::Object GetVersionCheckState() {
	json::Object result;
	result["enabled"] = false;
	result["checking"] = false;
	result["status"] = "disabled";
	result["current_version"] = std::string(GetAegisubLongVersionString());
	result["source"] = updates::releases_url;
	result["message"] = "Update checking is disabled in this build.";
	result["releases"] = json::Array{};
	return result;
}
#endif
