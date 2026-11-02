// Copyright (c) 2005, Rodrigo Braz Monteiro, Niels Martin Hansen
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

/// @file frame_main.cpp
/// @brief Main window creation and control management
/// @ingroup main_ui

#include "frame_main.h"

#include "gpui_bridge.h"
#include "include/aegisub/context.h"
#include "include/aegisub/menu.h"
#include "include/aegisub/toolbar.h"
#include "include/aegisub/hotkey.h"

#include "ass_file.h"
#include "async_video_provider.h"
#include "audio_controller.h"
#include "audio_box.h"
#include "base_grid.h"
#include "compat.h"
#include "command/command.h"
#include "dialog_detached_video.h"
#include "dialog_manager.h"
#include "libresrc/libresrc.h"
#include "main.h"
#include "options.h"
#include "project.h"
#include "subs_controller.h"
#include "subs_edit_box.h"
#include "ui_theme.h"
#include "utils.h"
#include "version.h"
#include "video_box.h"
#include "video_controller.h"
#include "video_display.h"

#include <libaegisub/dispatch.h>
#include <libaegisub/log.h>

#include <wx/dnd.h>
#include <wx/filename.h>
#include <wx/filefn.h>
#include <wx/msgdlg.h>
#include <wx/process.h>
#include <wx/stdpaths.h>
#include <wx/sizer.h>
#include <wx/statline.h>
#include <wx/statusbr.h>
#include <wx/sysopt.h>
#include <wx/toolbar.h>

enum {
	ID_APP_TIMER_STATUSCLEAR = 12002,
	ID_APP_LAUNCH_GPUI
};

#ifdef WITH_STARTUPLOG
#define StartupLog(a) MessageBox(0, a, "Neo Aegisub startup log", 0)
#else
#define StartupLog(a) LOG_I("frame_main/init") << a
#endif

/// Handle files drag and dropped onto Aegisub
class AegisubFileDropTarget final : public wxFileDropTarget {
	agi::Context *context;
public:
	AegisubFileDropTarget(agi::Context *context) : context(context) { }
	bool OnDropFiles(wxCoord, wxCoord, wxArrayString const& filenames) override {
		std::vector<agi::fs::path> files;
		for (wxString const& fn : filenames)
			files.push_back(from_wx(fn));
		agi::dispatch::Main().Async([=, this] { context->project->LoadList(files); });
		return true;
	}
};

FrameMain::FrameMain()
: wxFrame(nullptr, -1, "")
, context(std::make_unique<agi::Context>())
{
	StartupLog("Entering FrameMain constructor");
	Bind(wxEVT_END_PROCESS, [this](wxProcessEvent& event) {
		if (event.GetPid() != gpuiPid) return;
		gpuiPid = 0;
		delete gpuiProcess;
		gpuiProcess = nullptr;
#ifdef WITH_GPUI_DEFAULT
		Close();
#endif
	}, ID_APP_LAUNCH_GPUI);
	ui::ApplySurface(this, true);
	Bind(wxEVT_MENU_OPEN, [this](wxMenuEvent& event) {
		ui::ApplyAppearanceToMenuBar(GetMenuBar());
		event.Skip();
	});

	SetSize(FromDIP(wxSize(1200, 780)));

#ifdef __WXGTK__
	// XXX HACK XXX
	// We need to set LC_ALL to "" here for input methods to work reliably.
	setlocale(LC_ALL, "");

	// However LC_NUMERIC must be "C", otherwise some parsing fails.
	setlocale(LC_NUMERIC, "C");
#endif

	StartupLog("Initializing context controls");
	BindConnection(context->ass->AddCommitListener(&FrameMain::UpdateTitle, this));
	BindConnection(context->subsController->AddFileOpenListener(&FrameMain::OnSubtitlesOpen, this));
	BindConnection(context->subsController->AddFileSaveListener(&FrameMain::UpdateTitle, this));
	BindConnection(context->project->AddAudioProviderListener(&FrameMain::OnAudioOpen, this));
	BindConnection(context->project->AddVideoProviderListener(&FrameMain::OnVideoOpen, this));

	StartupLog("Initializing context frames");
	context->parent = this;
	context->frame = this;

	StartupLog("Apply saved Maximized state");
	if (OPT_GET("App/Maximized")->GetBool()) Maximize(true);

	StartupLog("Initialize toolbar");
	wxSystemOptions::SetOption("msw.remap", 0);
	BindConnection(OPT_SUB("App/Show Toolbar", &FrameMain::EnableToolBar, this));
	EnableToolBar(*OPT_GET("App/Show Toolbar"));

	StartupLog("Initialize menu bar");
	menu::GetMenuBar("main", this, (wxID_HIGHEST + 1) + 10000, context.get());
	ui::ApplyAppearanceToMenuBar(GetMenuBar());

	StartupLog("Create status bar");
	CreateStatusBar(2);
	ui::ApplySurface(GetStatusBar(), true);
	GetStatusBar()->SetMinHeight(FromDIP(28));

	StartupLog("Set icon");
#ifdef _WIN32
	SetIcon(wxICON(wxicon));
#else
	wxIcon icon;
	icon.CopyFromBitmap(GETIMAGE(wxicon));
	SetIcon(icon);
#endif

	StartupLog("Create views and inner main window controls");
	InitContents();
	BindConnection(OPT_SUB("Video/Detached/Enabled", &FrameMain::OnVideoDetach, this));

	StartupLog("Set up drag/drop target");
	SetDropTarget(new AegisubFileDropTarget(context.get()));

	StartupLog("Load default file");
	context->project->CloseSubtitles();

	StartupLog("Display main window");
	AddFullScreenButton(this);
#ifndef WITH_GPUI_DEFAULT
	Show();
#endif
	SetDisplayMode(1, 1);

	StartupLog("Leaving FrameMain constructor");
}

FrameMain::~FrameMain () {
	StopGpui();
	context->project->CloseAudio();
	context->project->CloseVideo();

	DestroyChildren();
}

void FrameMain::EnableToolBar(agi::OptionValue const& opt) {
	if (opt.GetBool()) {
		if (!GetToolBar()) {
			toolbar::AttachToolbar(this, "main", context.get(), "Default");
			GetToolBar()->Realize();
		}
	}
	else if (wxToolBar *old_tb = GetToolBar()) {
		SetToolBar(nullptr);
		delete old_tb;
		Layout();
	}
}

void FrameMain::InitContents() {
	StartupLog("Create background panel");
	auto Panel = new wxPanel(this, -1, wxDefaultPosition, wxDefaultSize, wxTAB_TRAVERSAL | wxCLIP_CHILDREN);
	Panel->SetName("AegisubMainCanvas");
	ui::ApplySurface(Panel, true);

	StartupLog("Create subtitles grid");
	context->subsGrid = new BaseGrid(Panel, context.get());

	StartupLog("Create video box");
	videoBox = new VideoBox(Panel, false, context.get());

	StartupLog("Create audio box");
	context->audioBox = audioBox = new AudioBox(Panel, context.get());

	StartupLog("Create subtitle editing box");
	auto EditBox = new SubsEditBox(Panel, context.get());

	StartupLog("Arrange main sizers");
	const int gutter = Panel->FromDIP(10);
	ToolsSizer = new wxBoxSizer(wxVERTICAL);
	ToolsSizer->Add(audioBox, 0, wxEXPAND | wxBOTTOM, gutter);
	ToolsSizer->Add(EditBox, 1, wxEXPAND);
	TopSizer = new wxBoxSizer(wxHORIZONTAL);
	TopSizer->Add(videoBox, 0, wxEXPAND | wxRIGHT, gutter);
	TopSizer->Add(ToolsSizer, 1, wxEXPAND);
	MainSizer = new wxBoxSizer(wxVERTICAL);
	MainSizer->Add(TopSizer, 0, wxEXPAND | wxALL, gutter);
	MainSizer->Add(context->subsGrid, 1, wxEXPAND | wxLEFT | wxRIGHT | wxBOTTOM, gutter);
	Panel->SetSizer(MainSizer);

	StartupLog("Perform layout");
	Layout();
	StartupLog("Leaving InitContents");
}

void FrameMain::SetDisplayMode(int video, int audio) {
	if (!IsShownOnScreen()) return;

	bool sv = false, sa = false;

	if (video == -1) sv = showVideo;
	else if (video)  sv = context->project->VideoProvider() && !context->dialog->Get<DialogDetachedVideo>();

	if (audio == -1) sa = showAudio;
	else if (audio)  sa = !!context->project->AudioProvider();

	// See if anything changed
	if (sv == showVideo && sa == showAudio) return;

	showVideo = sv;
	showAudio = sa;

	bool didFreeze = !IsFrozen();
	if (didFreeze) Freeze();

	context->videoController->Stop();

	TopSizer->Show(videoBox, showVideo, true);
	ToolsSizer->Show(audioBox, showAudio, true);

	MainSizer->Layout();
	Layout();

	if (didFreeze) Thaw();
}

void FrameMain::UpdateTitle() {
	wxString newTitle;
	if (context->subsController->IsModified()) newTitle << "* ";
	newTitle << context->subsController->Filename().filename().wstring();

#ifndef __WXMAC__
	newTitle << " - Neo Aegisub " << GetAegisubLongVersionString();
#endif

#if defined(__WXMAC__)
	// On Mac, set the mark in the close button
	OSXSetModified(context->subsController->IsModified());
#endif

	if (GetTitle() != newTitle) SetTitle(newTitle);
}

void FrameMain::OnVideoOpen(AsyncVideoProvider *provider) {
	if (!provider) {
		SetDisplayMode(0, -1);
		return;
	}

	Freeze();
	int vidx = provider->GetWidth(), vidy = provider->GetHeight();

	// Set zoom level based on video resolution and window size
	double zoom = context->videoDisplay->GetWindowZoom();
	wxSize windowSize = GetSize();
	if (vidx*3*zoom > windowSize.GetX()*4 || vidy*4*zoom > windowSize.GetY()*6)
		context->videoDisplay->SetWindowZoom(zoom * .25);
	else if (vidx*3*zoom > windowSize.GetX()*2 || vidy*4*zoom > windowSize.GetY()*3)
		context->videoDisplay->SetWindowZoom(zoom * .5);

	SetDisplayMode(1,-1);

	if (OPT_GET("Video/Detached/Enabled")->GetBool() && !context->dialog->Get<DialogDetachedVideo>())
		cmd::call("video/detach", context.get());
	Thaw();
}

void FrameMain::OnVideoDetach(agi::OptionValue const& opt) {
	if (opt.GetBool())
		SetDisplayMode(0, -1);
	else if (context->project->VideoProvider())
		SetDisplayMode(1, -1);
}

void FrameMain::StatusTimeout(wxString text,int ms) {
	SetStatusText(text,1);
	StatusClear.SetOwner(this, ID_APP_TIMER_STATUSCLEAR);
	StatusClear.Start(ms,true);
}

bool FrameMain::LaunchGpui() {
	// The workspace command can fire while the frontend from startup is still
	// running; spawning another one would leave two windows on the same bridge.
	if (gpuiPid)
		return true;
	if (!gpuiBridge)
		gpuiBridge = std::make_unique<GpuiBridge>(context.get());
	if (!gpuiBridge->Start()) {
		wxMessageBox(_("Could not start the GPUI prototype bridge."), _("GPUI prototype"), wxOK | wxICON_ERROR | wxCENTER, this);
		return false;
	}

	wxFileName executable(wxStandardPaths::Get().GetExecutablePath());
	wxFileName prototype(executable.GetPath(), "aegisub-gpui-prototype");
#ifdef _WIN32
	prototype.SetExt("exe");
#endif
	if (!prototype.FileExists()) {
		wxString sourceRoot = wxGetCwd();
#ifdef __WXMAC__
		if (executable.GetPath().Find("/Contents/MacOS") != wxNOT_FOUND)
			sourceRoot = executable.GetPath() + "/../../../../";
#endif
		wxFileName sourceTree(sourceRoot + "/gpui-prototype/target/release/aegisub-gpui-prototype");
#ifdef _WIN32
		sourceTree.SetExt("exe");
#endif
		sourceTree.Normalize(wxPATH_NORM_DOTS | wxPATH_NORM_ABSOLUTE);
		if (sourceTree.FileExists())
			prototype = sourceTree;
	}

	wxExecuteEnv env;
	wxGetEnvMap(&env.env);
	const wxString executablePath = prototype.GetFullPath();
	const std::string endpoint = gpuiBridge->Endpoint();
	const std::string token = gpuiBridge->Token();
	const wxScopedCharBuffer executableUtf8 = executablePath.utf8_str();
	const char *const args[] = {
		executableUtf8.data(),
		"--bridge", endpoint.c_str(),
		"--token", token.c_str(),
		nullptr
	};
	auto process = std::make_unique<wxProcess>(this, ID_APP_LAUNCH_GPUI);
	long const pid = wxExecute(args, wxEXEC_ASYNC, process.get(), &env);
	if (pid == 0) {
		wxMessageBox(_("Could not launch the GPUI prototype. Build it with cargo build --release --manifest-path gpui-prototype/Cargo.toml."), _("GPUI prototype"), wxOK | wxICON_ERROR | wxCENTER, this);
		return false;
	}
	if (gpuiProcess) gpuiProcess->Detach();
	if (gpuiPid && gpuiPid != pid)
		wxKill(gpuiPid, wxSIGTERM, nullptr, wxKILL_CHILDREN);
	gpuiPid = pid;
	gpuiProcess = process.release();
	return true;
}

void FrameMain::StopGpui() {
	if (gpuiProcess) {
		gpuiProcess->Detach();
		gpuiProcess = nullptr;
	}
	if (gpuiPid) {
		wxKill(gpuiPid, wxSIGTERM, nullptr, wxKILL_CHILDREN);
		gpuiPid = 0;
	}
	if (gpuiBridge) {
		gpuiBridge->Stop();
		gpuiBridge.reset();
	}
}

BEGIN_EVENT_TABLE(FrameMain, wxFrame)
	EVT_TIMER(ID_APP_TIMER_STATUSCLEAR, FrameMain::OnStatusClear)
	EVT_CLOSE(FrameMain::OnCloseWindow)
	EVT_CHAR_HOOK(FrameMain::OnKeyDown)
	EVT_MOUSEWHEEL(FrameMain::OnMouseWheel)
END_EVENT_TABLE()

void FrameMain::OnCloseWindow(wxCloseEvent &event) {
	wxEventBlocker blocker(this, wxEVT_CLOSE_WINDOW);

	context->videoController->Stop();
	context->audioController->Stop();

#ifdef WITH_GPUI_DEFAULT
	// Once the GPUI window is gone there is nobody left to answer a modal, and
	// this frame is hidden anyway: keep the edits in an autosave file instead of
	// blocking on a dialog the user cannot see.
	if (!gpui::FrontendAttached() && context->subsController->IsModified()) {
		context->subsController->AutoSave();
	}
	else
#endif
	// Ask user if he wants to save first
	if (context->subsController->TryToClose(event.CanVeto()) == wxCANCEL) {
		event.Veto();
#ifdef WITH_GPUI_DEFAULT
		if (!gpuiPid) LaunchGpui();
#endif
		return;
	}

	StopGpui();
	context->dialog.reset();

	// Store maximization state
	OPT_SET("App/Maximized")->SetBool(IsMaximized());

	Destroy();
}

void FrameMain::OnStatusClear(wxTimerEvent &) {
	SetStatusText("",1);
}

void FrameMain::OnAudioOpen(agi::AudioProvider *provider) {
	if (provider)
		SetDisplayMode(-1, 1);
	else
		SetDisplayMode(-1, 0);
}

void FrameMain::OnSubtitlesOpen() {
	UpdateTitle();
	SetDisplayMode(1, 1);
}

void FrameMain::OnKeyDown(wxKeyEvent &event) {
	hotkey::check("Main Frame", context.get(), event);
}

void FrameMain::OnMouseWheel(wxMouseEvent &evt) {
	ForwardMouseWheelEvent(this, evt);
}
