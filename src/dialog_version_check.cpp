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

#include "dialogs.h"
#include "release_manifest.h"
#include "version.h"

#include <mutex>

#ifdef WITH_UPDATE_CHECKER
#include "compat.h"
#include "gpui_bridge.h"
#include "options.h"

#include <libaegisub/cajun/reader.h>
#include <libaegisub/dispatch.h>

#include <ctime>
#include <curl/curl.h>
#include <functional>
#include <memory>
#include <sstream>
#include <stdexcept>
#include <vector>
#include <wx/button.h>
#include <wx/checkbox.h>
#include <wx/dialog.h>
#include <wx/event.h>
#include <wx/hyperlink.h>
#include <wx/intl.h>
#include <wx/sizer.h>
#include <wx/statline.h>
#include <wx/stattext.h>
#include <wx/string.h>
#include <wx/textctrl.h>


namespace {
std::mutex VersionCheckLock;
bool checking = false;
bool interactive_check = false;
std::string check_status = "idle";
std::string check_message;
std::optional<updates::Release> check_release;

struct AegisubUpdateDescription {
	std::string url;
	std::string friendly_name;
	std::string description;
};

class VersionCheckerResultDialog final : public wxDialog {
	void OnRemindMeLater(wxCommandEvent &evt);
	void OnClose(wxCloseEvent &evt);

	wxCheckBox *automatic_check_checkbox;

public:
	VersionCheckerResultDialog(wxString const& main_text, const std::vector<AegisubUpdateDescription> &updates);

	bool ShouldPreventAppExit() const override { return false; }
};

VersionCheckerResultDialog::VersionCheckerResultDialog(wxString const& main_text, const std::vector<AegisubUpdateDescription> &updates)
: wxDialog(nullptr, -1, _("Version Checker"))
{
	const int controls_width = 500;

	wxSizer *main_sizer = new wxBoxSizer(wxVERTICAL);

	wxStaticText *text = new wxStaticText(this, -1, main_text);
	text->Wrap(controls_width);
	main_sizer->Add(text, 0, wxBOTTOM|wxEXPAND, 6);

	for (auto const& update : updates) {
		main_sizer->Add(new wxStaticLine(this), 0, wxEXPAND|wxALL, 6);

		text = new wxStaticText(this, -1, to_wx(update.friendly_name));
		wxFont boldfont = text->GetFont();
		boldfont.SetWeight(wxFONTWEIGHT_BOLD);
		text->SetFont(boldfont);
		main_sizer->Add(text, 0, wxEXPAND|wxBOTTOM, 6);

		wxTextCtrl *descbox = new wxTextCtrl(this, -1, to_wx(update.description), wxDefaultPosition, wxSize(controls_width,60), wxTE_MULTILINE|wxTE_READONLY);
		main_sizer->Add(descbox, 0, wxEXPAND|wxBOTTOM, 6);

		main_sizer->Add(new wxHyperlinkCtrl(this, -1, to_wx(update.url), to_wx(update.url)), 0, wxALIGN_LEFT|wxBOTTOM, 6);
	}

	automatic_check_checkbox = new wxCheckBox(this, -1, _("&Auto Check for Updates"));
	automatic_check_checkbox->SetValue(OPT_GET("App/Auto/Check For Updates")->GetBool());

	wxButton *remind_later_button = nullptr;
	if (updates.size() > 0)
		remind_later_button = new wxButton(this, wxID_NO, _("Remind me again in a &week"));

	wxButton *close_button = new wxButton(this, wxID_OK, _("&Close"));
	SetAffirmativeId(wxID_OK);
	SetEscapeId(wxID_OK);

	if (updates.size())
		main_sizer->Add(new wxStaticLine(this), 0, wxEXPAND|wxALL, 6);
	main_sizer->Add(automatic_check_checkbox, 0, wxEXPAND|wxBOTTOM, 6);

	auto button_sizer = new wxStdDialogButtonSizer();
	button_sizer->AddButton(close_button);
	if (remind_later_button)
		button_sizer->AddButton(remind_later_button);
	button_sizer->Realize();
	main_sizer->Add(button_sizer, 0, wxEXPAND, 0);

	wxSizer *outer_sizer = new wxBoxSizer(wxVERTICAL);
	outer_sizer->Add(main_sizer, 0, wxALL|wxEXPAND, 12);

	SetSizerAndFit(outer_sizer);
	Centre();
	Show();

	Bind(wxEVT_BUTTON, std::bind(&VersionCheckerResultDialog::Close, this, false), wxID_OK);
	Bind(wxEVT_BUTTON, &VersionCheckerResultDialog::OnRemindMeLater, this, wxID_NO);
	Bind(wxEVT_CLOSE_WINDOW, &VersionCheckerResultDialog::OnClose, this);
}

void VersionCheckerResultDialog::OnRemindMeLater(wxCommandEvent &) {
	// In one week
	time_t new_next_check_time = time(nullptr) + 7*24*60*60;
	OPT_SET("Version/Next Check")->SetInt(new_next_check_time);

	Close();
}

void VersionCheckerResultDialog::OnClose(wxCloseEvent &) {
	OPT_SET("App/Auto/Check For Updates")->SetBool(automatic_check_checkbox->GetValue());
	Destroy();
}

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
	curl_easy_setopt(curl.get(), CURLOPT_URL, "https://api.github.com/repos/AlanWanco/Aegisub/releases?per_page=100");
	curl_easy_setopt(curl.get(), CURLOPT_HTTPHEADER, headers.get());
	curl_easy_setopt(curl.get(), CURLOPT_USERAGENT, "Neo Aegisub-update-checker/1.0");
	curl_easy_setopt(curl.get(), CURLOPT_CONNECTTIMEOUT, 10L);
	curl_easy_setopt(curl.get(), CURLOPT_TIMEOUT, 30L);
	curl_easy_setopt(curl.get(), CURLOPT_NOSIGNAL, 1L);
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
		std::vector<AegisubUpdateDescription> results;
		std::string status;
		std::string message;
		std::optional<updates::Release> release;
		try {
			release = DoCheck();
			status = release ? "available" : "current";
			message = from_wx(release ? _("An update to Neo Aegisub was found.") : _("There are no updates to Neo Aegisub."));
			if (release) {
				results.push_back({release->url, release->name, release->description});
			}
		}
		catch (std::exception const& error) {
			status = "error";
			message = std::string("There was an error checking for updates to Neo Aegisub: ") + error.what();
		}
		catch (...) {
			status = "error";
			message = "An unknown error occurred while checking for updates to Neo Aegisub.";
		}
		bool show;
		{
			std::lock_guard lock(VersionCheckLock);
			check_status = status;
			check_message = message;
			check_release = std::move(release);
			checking = false;
			show = interactive_check || !results.empty();
			interactive_check = false;
		}
		agi::dispatch::Main().Async([show, message = std::move(message), results = std::move(results)] {
			OPT_SET("Version/Next Check")->SetInt(time(nullptr) + 60*60);
			if (show && !gpui::FrontendAttached())
				new VersionCheckerResultDialog(to_wx(message), results);
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
