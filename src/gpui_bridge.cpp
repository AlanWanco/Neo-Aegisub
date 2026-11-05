// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

#include "gpui_bridge.h"

#include "gpui_loop.h"
#include "ass_style.h"

#include "grid_display_text.h"
#include "gpui_audio.h"
#include "gpui_automation.h"
#include "gpui_color.h"
#include "gpui_dialogs.h"
#include "gpui_command_catalog.h"
#include "gpui_document.h"
#include "gpui_translation.h"
#include "gpui_timing.h"
#include "gpui_search.h"
#include "gpui_video.h"
#include "gpui_visual.h"
#include "gpui_preferences.h"
#include "gpui_server.h"
#include "gpui_spelling.h"
#include "ui_theme.h"

#include "ass_dialogue.h"
#include "ass_file.h"
#include "async_video_provider.h"
#include "audio_controller.h"
#include "auto4_base.h"
#include "dialogs.h"
#include "command/command.h"
#include "command/edit.h"
#include "include/aegisub/context.h"
#include "options.h"
#include "project.h"
#include "selection_controller.h"
#include "utils.h"
#include "subtitle_format.h"
#include "subs_controller.h"
#include "text_selection_controller.h"
#include "video_controller.h"

#include <libaegisub/cajun/reader.h>
#include <libaegisub/cajun/visitor.h>
#include <libaegisub/charset_conv.h>
#include <libaegisub/log.h>


#include <algorithm>
#include <array>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <deque>
#include <iomanip>
#include <limits>
#include <map>
#include <mutex>
#include <random>
#include <sstream>
#include <string_view>
#include <vector>

namespace {

std::mutex notice_mutex;
struct PendingNotice {
	std::string title;
	std::string body;
	/// Option that turns this warning off; empty when the notice is one-shot.
	std::string suppress;
	/// Tells the front end which action row to draw; empty for a plain notice.
	std::string kind;
	/// Answer buttons; empty for an ordinary acknowledgement.
	std::vector<std::string> choices;
};
std::vector<PendingNotice> pending_notices;
/// Answer handlers for the questions that are currently on screen, oldest
/// first. A single slot is not enough: loading a video can raise the YCbCr and
/// the layout resolution questions in the same pass, and the front end answers
/// them in the order it shows them.
std::deque<std::function<void(int)>> pending_choices;
std::vector<std::string> pending_commands;
/// Text for the front end's status line; empty until something publishes one.
std::string pending_status;

struct ProgressState {
	bool active = false;
	std::string title;
	std::string message;
	double fraction = 0.;
	bool cancellable = true;
	bool cancelled = false;
};
ProgressState progress_state;

int attached_clients = 0;

void AttachClient() {
	std::lock_guard<std::mutex> lock(notice_mutex);
	++attached_clients;
}

void DetachClient() {
	std::lock_guard<std::mutex> lock(notice_mutex);
	if (attached_clients > 0) --attached_clients;
}

std::string make_token() {
	std::array<unsigned char, 32> bytes{};
	std::random_device random;
	for (auto& byte : bytes)
		byte = static_cast<unsigned char>(random());

	std::ostringstream out;
	out << std::hex << std::setfill('0');
	for (auto byte : bytes)
		out << std::setw(2) << static_cast<unsigned>(byte);
	return out.str();
}

std::string quote_json(std::string_view value) {
	std::ostringstream out;
	out << '"';
	for (unsigned char ch : value) {
		switch (ch) {
			case '"': out << "\\\""; break;
			case '\\': out << "\\\\"; break;
			case '\b': out << "\\b"; break;
			case '\f': out << "\\f"; break;
			case '\n': out << "\\n"; break;
			case '\r': out << "\\r"; break;
			case '\t': out << "\\t"; break;
			default:
				if (ch < 0x20)
					out << "\\u00" << std::hex << std::setw(2) << static_cast<unsigned>(ch) << std::dec;
				else
					out << static_cast<char>(ch);
		}
	}
	out << '"';
	return out.str();
}


struct JsonStringCheck final : json::ConstVisitor {
	bool is_string = false;
	void Visit(const json::Array&) override { }
	void Visit(const json::Object&) override { }
	void Visit(int64_t) override { }
	void Visit(double) override { }
	void Visit(const json::String&) override { is_string = true; }
	void Visit(bool) override { }
	void Visit(const json::Null&) override { }
};

bool is_json_string(json::UnknownElement const& element) {
	JsonStringCheck check;
	element.Accept(check);
	return check.is_string;
}
}

bool gpui::FrontendAttached() {
	std::lock_guard<std::mutex> lock(notice_mutex);
	return attached_clients > 0;
}

void gpui::PushNotice(std::string title, std::string body) {
	std::lock_guard<std::mutex> lock(notice_mutex);
	if (attached_clients == 0) return;
	pending_notices.push_back({std::move(title), std::move(body), {}, {}});
}

void gpui::PushNotice(std::string title, std::string body, std::string suppress_option) {
	std::lock_guard<std::mutex> lock(notice_mutex);
	if (attached_clients == 0) return;
	pending_notices.push_back({std::move(title), std::move(body), std::move(suppress_option), {}});
}

void gpui::PushNotice(std::string title, std::string body, std::string suppress_option, std::string kind) {
	std::lock_guard<std::mutex> lock(notice_mutex);
	if (attached_clients == 0) return;
	pending_notices.push_back({std::move(title), std::move(body), std::move(suppress_option), std::move(kind)});
}

void gpui::PushChoice(std::string title, std::string body, std::string suppress_option,
                      std::string kind, std::vector<std::string> choices,
                      std::function<void(int)> apply) {
	std::lock_guard<std::mutex> lock(notice_mutex);
	if (attached_clients == 0) return;
	pending_notices.push_back({std::move(title), std::move(body), std::move(suppress_option),
		std::move(kind), std::move(choices)});
	pending_choices.push_back(std::move(apply));
}

void gpui::PushCommand(std::string name) {
	std::lock_guard<std::mutex> lock(notice_mutex);
	if (attached_clients == 0) return;
	pending_commands.push_back(std::move(name));
}

int gpui::AskChoice(std::string title, std::string body, std::string suppress_option,
                    std::string kind, std::vector<std::string> choices) {
	if (!FrontendAttached()) return -1;
	int answer = -1;
	bool answered = false;
	PushChoice(std::move(title), std::move(body), std::move(suppress_option), std::move(kind),
		std::move(choices), [&](int choice) {
			answer = choice;
			answered = true;
		});
	while (!answered) EventLoop::Current().Pump(8);
	return answer;
}

void gpui::PushStatus(std::string text) {
	std::lock_guard<std::mutex> lock(notice_mutex);
	pending_status = std::move(text);
}

void gpui::PushProgress(std::string title, std::string message, double fraction,
                        bool cancellable) {
	std::lock_guard<std::mutex> lock(notice_mutex);
	if (attached_clients == 0) return;
	// A new task starts with a clean cancel flag; a task that keeps publishing
	// must not wipe a cancel the user already pressed.
	if (!progress_state.active) progress_state.cancelled = false;
	progress_state.active = true;
	progress_state.title = std::move(title);
	progress_state.message = std::move(message);
	progress_state.fraction = fraction;
	progress_state.cancellable = cancellable;
}

void gpui::ClearProgress() {
	std::lock_guard<std::mutex> lock(notice_mutex);
	progress_state = ProgressState{};
}

bool gpui::ProgressCancelled() {
	std::lock_guard<std::mutex> lock(notice_mutex);
	return progress_state.cancelled;
}

struct GpuiBridge::Impl final : gpui::SocketServer::Handler {
	agi::Context *context;
	gpui::VideoFrames video_frames;
	gpui::Audio audio;
	gpui::Visual visual;
	gpui::Spelling spelling;
	gpui::SocketServer server{this};
	unsigned short port = 0;
	uint64_t video_epoch = 0;
	uint64_t revision = 1;
	uint64_t details_revision = 1;
	agi::signal::Connection commit_connection;
	AsyncVideoProvider *observed_provider = nullptr;
	std::string token = make_token();

	explicit Impl(agi::Context *context) : context(context), video_frames(context), audio(context), visual(context), spelling(context) {
		commit_connection = context->ass->AddCommitListener([this](int type, const AssDialogue *) {
			++revision;
			if (type == AssFile::COMMIT_NEW || (type & (AssFile::COMMIT_STYLES | AssFile::COMMIT_SCRIPTINFO)))
				++details_revision;
		});
	}

	~Impl() { Stop(); }

	void OnConnect(int) override { AttachClient(); }

	void OnDisconnect(int) override { DetachClient(); }

	void OnRequest(int client, std::string const& request) override {
		try {
			Handle(request, client);
		}
		catch (std::exception const& error) {
			Reply(client, "{\"ok\":false,\"error\":" + quote_json(error.what()) + "}");
		}
		catch (agi::Exception const& error) {
			Reply(client, "{\"ok\":false,\"error\":" + quote_json(error.GetMessage()) + "}");
		}
		catch (...) {
			Reply(client, R"({"ok":false,"error":"unknown bridge error"})");
		}
	}

	void Reply(int client, std::string response) {
		server.Send(client, std::move(response));
	}

	/// Raw replies send `head`, a newline, then `body` bytes verbatim. The body is
	/// moved out of the reply: never copied, escaped, or parsed as JSON.
	void Reply(int client, gpui::RawReply reply) {
		server.Send(client, std::move(reply.head),
		            std::string_view(reinterpret_cast<char const *>(reply.body.data()), reply.body.size()));
	}

	void Stop() {
		video_frames.Close();
		server.Close();
		observed_provider = nullptr;
		++video_epoch;
		port = 0;
	}

	void Handle(std::string const& request, int client) {
		std::istringstream stream(request);
		json::UnknownElement parsed;
		json::Reader::Read(parsed, stream);
		auto const& object = static_cast<json::Object const&>(parsed);
		auto token_it = object.find("token");
		if (token_it == object.end() || static_cast<std::string const&>(token_it->second) != token) {
			Reply(client, R"({"ok":false,"error":"unauthorized"})");
			return;
		}

		auto op_it = object.find("op");
		if (op_it == object.end()) {
			Reply(client, R"({"ok":false,"error":"missing operation"})");
			return;
		}
		std::string const& op = op_it->second;

		// Automation scripts ask for trust through the front end, so they are
		// created on the first request: by then a client is connected and the
		// prompt has a window to land in.
		if (!config::global_scripts)
			config::global_scripts = new Automation4::AutoloadScriptManager(
				OPT_GET("Path/Automation/Autoload")->GetString());

		if (op == "spelling") { Reply(client, spelling.Request(object)); return; }
		if (op == "file_tools") { Reply(client, gpui::FileTools(object)); return; }
		if (op == "save_media") { Reply(client, gpui::SaveMedia(context, object)); return; }
		if (op == "preferences") { Reply(client, gpui::Preferences(object)); return; }
		if (op == "hotkeys") { Reply(client, gpui::Hotkeys(object)); return; }
		if (op == "progress_cancel") {
			{
				std::lock_guard<std::mutex> lock(notice_mutex);
				if (progress_state.active) progress_state.cancelled = true;
			}
			Reply(client, "{\"ok\":true}");
			return;
		}
		if (op == "snapshot") { Reply(client, Snapshot()); return; }
		if (op == "playback") { Reply(client, PlaybackSnapshot()); return; }
		if (op == "video_settings") { Reply(client, gpui::VideoSettings(context, object)); return; }
		if (op == "video_frame") { Reply(client, video_frames.Frame(object)); return; }
		if (op == "visual") { Reply(client, visual.Interact(object)); return; }
		if (op == "audio_render") { Reply(client, audio.Render(object)); return; }
		if (op == "audio_action") { Reply(client, audio.Interact(object)); return; }
		if (op == "automation_manager") { Reply(client, gpui::AutomationManager(context, object)); return; }
		if (op == "dialog") {
		Reply(client, gpui::Dialog(context, object));
		return;
	}
	if (op == "color") { Reply(client, gpui::ColorPanel(context, object)); return; }
		if (op == "catalog") { Reply(client, gpui::CommandCatalog(context, visual)); return; }
		if (op == "attachments") {
			if (auto expected = object.find("revision"); expected != object.end()) {
				if (static_cast<json::Integer const&>(expected->second) != static_cast<int64_t>(revision)) { Reply(client, R"({"ok":false,"error":"document changed; refresh attachments first"})"); return; }
			}
			Reply(client, gpui::Attachments(context, object, revision)); return;
		}
		if (op == "resample") { Reply(client, gpui::Resample(context, object)); return; }
		if (op == "document") { Reply(client, gpui::DocumentDetails(context)); return; }
		if (op == "script_info") { Reply(client, gpui::EditScriptInfo(context, object)); return; }
		if (op == "style") { Reply(client, gpui::EditStyle(context, object)); return; }
		if (op == "style_preview") { Reply(client, gpui::StylePreview(context, object)); return; }
		if (op == "styling") { Reply(client, gpui::Styling(context, object)); return; }
		if (op == "style_library") { Reply(client, gpui::StyleLibrary(context, object)); return; }
		if (op == "translation") { Reply(client, gpui::Translation(context, object)); return; }
		if (op == "timing_processor") { Reply(client, gpui::TimingProcessor(context, object)); return; }
		if (op == "paste_over") {
			if (auto action = object.find("fields"); action != object.end()) {
				std::vector<bool> fields;
				for (auto const& value : static_cast<json::Array const&>(action->second)) fields.push_back(static_cast<json::Boolean const&>(value));
				auto changed = cmd::PasteLinesOver(context, std::move(fields));
				Reply(client, "{\"ok\":true,\"changed\":" + std::to_string(changed) + "}");
			}
			else {
				auto fields = OPT_GET("Tool/Paste Lines Over/Fields")->GetListBool();
				fields.resize(11, false);
				std::string response = "{\"ok\":true,\"fields\":[";
				for (size_t i = 0; i < fields.size(); ++i) { if (i) response += ','; response += fields[i] ? "true" : "false"; }
				Reply(client, response + "]}");
			}
			return;
		}
		if (op == "search") { Reply(client, gpui::Search(context, object)); return; }
		if (op == "shift_times") { Reply(client, gpui::ShiftTimesPanel(context, object)); return; }
		if (op == "select") { Reply(client, Select(object)); return; }
		if (op == "edit") { Reply(client, Edit(object)); return; }
		if (op == "line_fields") { Reply(client, LineFields(object)); return; }
		if (op == "seek") { Reply(client, Seek(object)); return; }
		if (op == "linked_files") {
			auto value = object.find("apply");
			bool const apply = value != object.end() && static_cast<json::Boolean const&>(value->second);
			context->project->AnswerLinkedFiles(apply);
			Reply(client, R"({"ok":true})");
			return;
		}
		if (op == "notice_choice") {
			auto index = object.find("index");
			int const choice = index == object.end() ? -1 : static_cast<int>(static_cast<json::Integer const&>(index->second));
			std::function<void(int)> apply;
			{
				std::lock_guard<std::mutex> lock(notice_mutex);
				if (!pending_choices.empty()) {
					apply = std::move(pending_choices.front());
					pending_choices.pop_front();
				}
			}
			if (apply) apply(choice);
			Reply(client, R"({"ok":true})");
			return;
		}
		if (op == "notice_discard") {
			// The user answered "don't save": put the document back to what is on
			// disk so nothing is lost silently. An untitled document has no file to
			// fall back to, so it becomes a blank one, which is what wx does when
			// the caller replaces it anyway.
			if (context->subsController->HasFile())
				context->project->LoadSubtitles(context->subsController->Filename());
			else
				context->subsController->Close();
			Reply(client, R"({"ok":true})");
			return;
		}
		if (op == "notice_suppress") {
			// Only the warnings that offer the switch may be silenced here; the
			// socket is not a licence to write arbitrary preferences.
			static const std::map<std::string, std::string> suppressible = {
				{"Video/No LayoutRes in Script", "Video/No LayoutRes in Script"},
				{"Video/No YCbCr Matrix in Script", "Video/No YCbCr Matrix in Script"},
			};
			std::string const& key = object.at("key");
			if (!suppressible.count(key)) throw std::runtime_error("unsupported notice option: " + key);
			OPT_SET(key)->SetInt(0);
			// Write through now: the switch is a preference, and losing it when
			// the app is killed before the next options flush would bring the
			// prompt straight back.
			config::opt->Flush();
			Reply(client, R"({"ok":true})");
			return;
		}
		if (op == "command") { Reply(client, Command(object)); return; }
		if (op == "open") { Reply(client, Open(object)); return; }
		if (op == "save_as") { Reply(client, SaveAs(object)); return; }
		Reply(client, R"({"ok":false,"error":"unsupported operation"})");
	}

	std::string PlaybackSnapshot() {
		auto provider = context->project->VideoProvider();
		if (provider != observed_provider) {
			observed_provider = provider;
			++video_epoch;
		}
		std::ostringstream out;
		out << "{\"ok\":true"
		    << ",\"video\":" << (provider ? "true" : "false")
		    << ",\"video_epoch\":" << video_epoch
		    << ",\"playing\":" << (context->videoController->IsPlaying() ? "true" : "false")
		    << ",\"frame\":" << context->videoController->GetFrameN()
		    << ",\"video_time\":" << (provider ? context->videoController->TimeAtFrame(context->videoController->GetFrameN()) : 0)
		    << ",\"audio_playing\":" << (context->audioController->IsPlaying() ? "true" : "false")
		    << ",\"audio_position\":" << context->audioController->GetPlaybackPosition()
		    << '}';
		return out.str();
	}

	std::string Snapshot() {
		auto current_provider = context->project->VideoProvider();
		if (current_provider != observed_provider) {
			observed_provider = current_provider;
			++video_epoch;
		}
		std::ostringstream out;
		out << "{\"ok\":true,\"file\":"
		    << quote_json(context->subsController->Filename().string())
		    << ",\"modified\":" << (context->subsController->IsModified() ? "true" : "false")
		    << ",\"dark\":" << (ui::IsDark() ? "true" : "false")
		    << ",\"show_toolbar\":" << (OPT_GET("App/Show Toolbar")->GetBool() ? "true" : "false")
		    << ",\"appearance\":" << gpui::Appearance()
		    << ",\"can_save\":" << (context->subsController->CanSave() ? "true" : "false")
		    << ",\"revision\":" << revision << ",\"details_revision\":" << details_revision
		    << ",\"video\":" << (context->project->VideoProvider() ? "true" : "false")
		    << ",\"timecodes\":" << (context->project->Timecodes().IsLoaded() ? "true" : "false")
		    << ",\"playing\":" << (context->videoController->IsPlaying() ? "true" : "false")
		    << ",\"frame\":" << context->videoController->GetFrameN()
		    << ",\"video_epoch\":" << video_epoch
		    << ",\"frame_count\":";
		if (auto provider = context->project->VideoProvider())
			out << provider->GetFrameCount();
		else
			out << 0;
		out << ",\"fps\":";
		if (auto provider = context->project->VideoProvider())
			out << provider->GetFPS().FPS();
		else
			out << "null";
		out << ",\"video_time\":" << (current_provider ? context->videoController->TimeAtFrame(context->videoController->GetFrameN()) : 0)
		    << ",\"video_duration\":" << (current_provider ? context->videoController->TimeAtFrame(current_provider->GetFrameCount() - 1, agi::vfr::END) : 0)
		    << ",\"playback_rate\":" << context->videoController->GetPlaybackRate()
		    << ",\"aspect\":" << (current_provider ? context->videoController->GetAspectRatioValue() : 1.0)
		    << ",\"video_height\":" << (current_provider ? current_provider->GetHeight() : 0)
		    << ",\"video_scroll\":" << OPT_GET("Video/Scroll Action")->GetInt()
		    << ",\"video_ctrl_scroll\":" << OPT_GET("Video/Ctrl Scroll Action")->GetInt()
		    << ",\"video_shift_scroll\":" << OPT_GET("Video/Shift Scroll Action")->GetInt()
		    << ",\"video_overscan\":" << (OPT_GET("Video/Overscan Mask")->GetBool() ? "true" : "false");
		out << ",\"styles\":[";
		{
			bool first_style = true;
			for (auto const& style : context->ass->Styles) {
				if (!first_style) out << ',';
				first_style = false;
				out << quote_json(style.name);
			}
		}
		out << ']';
		out << ",\"audio\":" << audio.Snapshot();
		out << ",\"notices\":[";
		{
			std::lock_guard<std::mutex> lock(notice_mutex);
			bool first_notice = true;
			for (auto const& notice : pending_notices) {
				if (!first_notice) out << ',';
				first_notice = false;
				out << "{\"title\":" << quote_json(notice.title)
					<< ",\"body\":" << quote_json(notice.body);
				if (!notice.suppress.empty())
					out << ",\"suppress\":" << quote_json(notice.suppress);
				if (!notice.kind.empty())
					out << ",\"kind\":" << quote_json(notice.kind);
				if (!notice.choices.empty()) {
					out << ",\"choices\":[";
					for (size_t i = 0; i < notice.choices.size(); ++i) {
						if (i) out << ',';
						out << quote_json(notice.choices[i]);
					}
					out << ']';
				}
				if (!notice.kind.empty())
					out << ",\"kind\":" << quote_json(notice.kind);
				out << '}';
			}
			pending_notices.clear();
		}
		out << ']';
		out << ",\"commands\":[";
		{
			std::lock_guard<std::mutex> lock(notice_mutex);
			bool first_command = true;
			for (auto const& name : pending_commands) {
				if (!first_command) out << ',';
				first_command = false;
				out << quote_json(name);
			}
			pending_commands.clear();
		}
		out << ']';
		out << ",\"status\":";
		{
			std::lock_guard<std::mutex> lock(notice_mutex);
			out << quote_json(pending_status);
		}
		out << ",\"progress\":";
		{
			std::lock_guard<std::mutex> lock(notice_mutex);
			if (progress_state.active) {
				out << "{\"title\":" << quote_json(progress_state.title)
				    << ",\"message\":" << quote_json(progress_state.message)
				    << ",\"fraction\":" << progress_state.fraction
				    << ",\"cancellable\":" << (progress_state.cancellable ? "true" : "false")
				    << '}';
			}
			else
				out << "null";
		}
		out << ",\"active\":";
		auto active = context->selectionController->GetActiveLine();
		out << (active ? active->Id : -1) << ",\"selected\":[";
		bool first_selected = true;
		for (auto line : context->selectionController->GetSortedSelection()) {
			if (!first_selected) out << ',';
			first_selected = false;
			out << line->Id;
		}
		out << "],\"text_selection\":[" << context->textSelectionController->GetSelectionStart()
		    << ',' << context->textSelectionController->GetSelectionEnd() << "]";
		out << ",\"lines\":[";
		bool first = true;
		for (auto const& line : context->ass->Events) {
			if (!first) out << ',';
			first = false;
			out << "{\"id\":" << line.Id
			    << ",\"row\":" << line.Row
			    << ",\"start\":" << static_cast<int>(line.Start)
			    << ",\"end\":" << static_cast<int>(line.End)
			    << ",\"start_frame\":" << (current_provider ? context->videoController->FrameAtTime(line.Start, agi::vfr::START) : 0)
			    << ",\"end_frame\":" << (current_provider ? context->videoController->FrameAtTime(line.End, agi::vfr::END) : 0)
			    << ",\"layer\":" << line.Layer
			    << ",\"comment\":" << (line.Comment ? "true" : "false")
			    << ",\"style\":" << quote_json(line.Style.get())
			    << ",\"actor\":" << quote_json(line.Actor.get())
			    << ",\"effect\":" << quote_json(line.Effect.get())
			    << ",\"margin\":[" << line.Margin[0] << ',' << line.Margin[1] << ',' << line.Margin[2] << ']'
			    << ",\"display_text\":" << quote_json(GridDisplayText(line))
			    << ",\"text\":" << quote_json(line.Text.get()) << '}';
		}
		out << "]}";
		return out.str();
	}


	std::string Select(json::Object const& object) {
		auto id_it = object.find("id");
		if (id_it == object.end())
			return R"({"ok":false,"error":"missing line id"})";
		int id = static_cast<int>(static_cast<json::Integer const&>(id_it->second));
		Selection selected;
		if (auto ids = object.find("ids"); ids != object.end()) {
			for (auto const& item : static_cast<json::Array const&>(ids->second)) {
				int const selected_id = static_cast<int>(static_cast<json::Integer const&>(item));
				auto line = std::find_if(context->ass->Events.begin(), context->ass->Events.end(),
					[&](auto const& line) { return line.Id == selected_id; });
				if (line == context->ass->Events.end())
					return R"({"ok":false,"error":"selected line no longer exists"})";
				selected.insert(&*line);
			}
		}
		for (auto& line : context->ass->Events) {
			if (line.Id != id) continue;
			selected.insert(&line);
			context->selectionController->SetSelectionAndActive(std::move(selected), &line);
			if (auto seek = object.find("double_click"); seek != object.end()
				&& static_cast<bool const&>(seek->second)
				&& OPT_GET("Video/Seek On Double Click")->GetBool()
				&& context->project->VideoProvider())
				context->videoController->JumpToTime(line.Start);
			return R"({"ok":true})";
		}
		return R"({"ok":false,"error":"line no longer exists"})";
	}

	std::string Edit(json::Object const& object) {
		auto id_it = object.find("id");
		auto text_it = object.find("text");
		if (id_it == object.end() || text_it == object.end())
			return R"({"ok":false,"error":"missing line id or text"})";
		int id = static_cast<int>(static_cast<json::Integer const&>(id_it->second));
		std::string const& text = text_it->second;
		Selection selected;
		if (auto ids = object.find("ids"); ids != object.end()) {
			for (auto const& item : static_cast<json::Array const&>(ids->second)) {
				int const selected_id = static_cast<int>(static_cast<json::Integer const&>(item));
				auto line = std::find_if(context->ass->Events.begin(), context->ass->Events.end(),
					[&](auto const& line) { return line.Id == selected_id; });
				if (line == context->ass->Events.end())
					return R"({"ok":false,"error":"selected line no longer exists"})";
				selected.insert(&*line);
			}
		}
		auto active = std::find_if(context->ass->Events.begin(), context->ass->Events.end(),
			[&](auto const& line) { return line.Id == id; });
		if (active == context->ass->Events.end())
			return R"({"ok":false,"error":"line no longer exists"})";
		selected.insert(&*active);
		bool changed = false;
		for (auto line : selected) {
			if (line->Text == text) continue;
			line->Text = text;
			changed = true;
		}
		if (changed)
			context->ass->Commit(_("GPUI subtitle edit"), AssFile::COMMIT_DIAG_TEXT, -1,
				selected.size() == 1 ? *selected.begin() : nullptr);
		return changed ? R"({"ok":true})" : R"({"ok":true,"unchanged":true})";
	}

	std::string LineFields(json::Object const& object) {
		auto ids = object.find("ids");
		auto fields_it = object.find("fields");
		if (ids == object.end() || fields_it == object.end())
			return R"({"ok":false,"error":"missing line ids or fields"})";
		std::vector<AssDialogue *> lines;
		for (auto const& item : static_cast<json::Array const&>(ids->second)) {
			int const id = static_cast<int>(static_cast<json::Integer const&>(item));
			auto found = std::find_if(context->ass->Events.begin(), context->ass->Events.end(),
				[&](auto const& line) { return line.Id == id; });
			if (found == context->ass->Events.end())
				return R"({"ok":false,"error":"line no longer exists"})";
			if (std::find(lines.begin(), lines.end(), &*found) == lines.end()) lines.push_back(&*found);
		}
		if (lines.empty()) return R"({"ok":false,"error":"no lines selected"})";
		auto const& fields = static_cast<json::Object const&>(fields_it->second);
		// Parse the complete patch before changing the document.
		AssDialogueBase patch = *lines.front();
		int commit_type = 0;
		for (auto const& [name, value] : fields) {
			if (name == "style" || name == "actor" || name == "effect") {
				std::string const& text = value;
				if (name == "style") {
					if (!context->ass->GetStyle(text)) return R"({"ok":false,"error":"style does not exist"})";
					patch.Style = text;
				}
				if (name == "actor") patch.Actor = text;
				if (name == "effect") patch.Effect = text;
				commit_type |= AssFile::COMMIT_DIAG_META;
			}
			else if (name == "comment") {
				patch.Comment = static_cast<json::Boolean const&>(value);
				commit_type |= AssFile::COMMIT_DIAG_META;
			}
			else if (name == "start" || name == "end" || name == "layer" || name.starts_with("margin_")) {
				int64_t const number = static_cast<json::Integer const&>(value);
				int64_t const lower = name.starts_with("margin_") ? -9999 : 0;
				int64_t const upper = name == "start" || name == "end" ? std::numeric_limits<int>::max() - 10 : 99999;
				if (number < lower || number > upper) return R"({"ok":false,"error":"line field is out of range"})";
				if (name == "start") { patch.Start = number; commit_type |= AssFile::COMMIT_DIAG_TIME; }
				else if (name == "end") { patch.End = number; commit_type |= AssFile::COMMIT_DIAG_TIME; }
				else {
					commit_type |= AssFile::COMMIT_DIAG_META;
					if (name == "layer") patch.Layer = number;
					else if (name == "margin_left") patch.Margin[0] = number;
					else if (name == "margin_right") patch.Margin[1] = number;
					else if (name == "margin_vertical") patch.Margin[2] = number;
					else return R"({"ok":false,"error":"unknown line field"})";
				}
			}
			else return R"({"ok":false,"error":"unknown line field"})";
		}
		if (!commit_type) return R"({"ok":true,"unchanged":true})";
		for (auto line : lines) {
			int const start = fields.count("start") ? static_cast<int>(patch.Start) : static_cast<int>(line->Start);
			int const end = fields.count("end") ? static_cast<int>(patch.End) : static_cast<int>(line->End);
			if ((fields.count("start") || fields.count("end")) && end < start)
				return R"({"ok":false,"error":"end time must not precede start time"})";
		}
		bool changed = false;
		for (auto line : lines) {
			if (fields.count("start") && line->Start != patch.Start) { line->Start = patch.Start; changed = true; }
			if (fields.count("end") && line->End != patch.End) { line->End = patch.End; changed = true; }
			if (fields.count("layer") && line->Layer != patch.Layer) { line->Layer = patch.Layer; changed = true; }
			if (fields.count("style") && line->Style != patch.Style) { line->Style = patch.Style; changed = true; }
			if (fields.count("actor") && line->Actor != patch.Actor) { line->Actor = patch.Actor; changed = true; }
			if (fields.count("effect") && line->Effect != patch.Effect) { line->Effect = patch.Effect; changed = true; }
			if (fields.count("comment") && line->Comment != patch.Comment) { line->Comment = patch.Comment; changed = true; }
			const char *margin_keys[] = {"margin_left", "margin_right", "margin_vertical"};
			for (size_t i = 0; i < 3; ++i) {
				if (fields.count(margin_keys[i]) && line->Margin[i] != patch.Margin[i]) {
					line->Margin[i] = patch.Margin[i]; changed = true;
				}
			}
		}
		if (changed) context->ass->Commit(_("GPUI line properties"), commit_type);
		return changed ? R"({"ok":true})" : R"({"ok":true,"unchanged":true})";
	}

	std::string Seek(json::Object const& object) {
		auto frame_it = object.find("frame");
		if (frame_it == object.end())
			return R"({"ok":false,"error":"missing frame"})";
		int64_t const frame = static_cast<json::Integer const&>(frame_it->second);
		if (!context->project->VideoProvider()) return R"({"ok":false,"error":"no video is loaded"})";
		int const target = static_cast<int>(std::clamp<int64_t>(frame, 0, context->project->VideoProvider()->GetFrameCount() - 1));
		context->videoController->JumpToFrame(target);
		return R"({"ok":true})";
	}

	std::string Open(json::Object const& object) {
		auto kind_it = object.find("kind");
		auto path_it = object.find("path");
		if (kind_it == object.end() || path_it == object.end())
			return R"({"ok":false,"error":"missing file kind or path"})";
		if (!is_json_string(path_it->second) || !is_json_string(kind_it->second))
			return R"({"ok":false,"error":"file kind and path must be strings"})";
		std::string const& kind = kind_it->second;
		agi::fs::path path(static_cast<std::string const&>(path_it->second));
		if (kind == "subtitle") {
			if (auto recovery = object.find("recovery"); recovery != object.end() && static_cast<json::Boolean const&>(recovery->second)) {
				auto versions = GetAutosaveFiles();
				if (std::none_of(versions.begin(), versions.end(), [&](auto const& file) { return std::any_of(file.versions.begin(), file.versions.end(), [&](auto const& version) { return version.filename == path.string(); }); }))
					return R"({"ok":false,"error":"selected recovery version is no longer available"})";
			}
			if (!agi::fs::FileExists(path)) return R"({"ok":false,"error":"subtitle file is no longer available"})";
			std::string encoding;
			if (auto item = object.find("encoding"); item != object.end()) {
				encoding = static_cast<std::string const&>(item->second);
				if (!encoding.empty()) {
					// The menu uses display names, which IconvWrapper resolves to
					// real encoding names before validating them.
					agi::charset::IconvWrapper validate(encoding.c_str(), "UTF-8", false);
				}
			}
			if (context->subsController->TryToClose() == -1)
				return R"({"ok":false,"error":"opening subtitles was cancelled"})";
			context->project->LoadSubtitles(path, encoding);
			return R"({"ok":true})";
		}
		if (kind == "video") {
			context->project->LoadVideo(path);
			return R"({"ok":true})";
		}
		if (kind == "audio") {
			context->project->LoadAudio(path);
			return R"({"ok":true})";
		}
		if (kind == "keyframes") {
			context->project->LoadKeyframes(path);
			return R"({"ok":true})";
		}
		if (kind == "timecodes") {
			context->project->LoadTimecodes(path);
			return R"({"ok":true})";
		}
		return R"({"ok":false,"error":"unsupported file kind"})";
	}

	std::string SaveAs(json::Object const& object) {
		auto path_it = object.find("path");
		if (path_it == object.end())
			return R"({"ok":false,"error":"missing save path"})";
		try {
			if (!is_json_string(path_it->second))
				return R"({"ok":false,"error":"save path must be a string"})";
			std::string const& path = path_it->second;
			if (agi::fs::FileExists(agi::fs::path(path))) {
				// A modal prompt inside the bridge handler would block the front
				// end that is waiting for this response.
				gpui::PushNotice(_("Confirm Save As"),
					agi::format(_("The file %s already exists. Replace it?"), path) +
					"\n\n" + _("Pick a different name, or delete the old file first."));
				return R"({"ok":false,"error":"save was cancelled"})";
			}
			context->subsController->Save(agi::fs::path(path));
			return R"({"ok":true})";
		}
		catch (agi::Exception const& error) {
			return "{\"ok\":false,\"error\":" + quote_json(error.GetMessage()) + "}";
		}
		catch (std::exception const& error) {
			return "{\"ok\":false,\"error\":" + quote_json(error.what()) + "}";
		}
	}

	std::string Command(json::Object const& object) {
		// Commands are how the GPUI window flips preferences (hide overrides,
		// spectrum, overscan…). Those writes used to wait for a clean exit, so a
		// killed process lost them; write the file as soon as the command actually
		// changed an option, and stay quiet when it changed nothing.
		struct FlushChangedOptions {
			uint64_t const before = agi::OptionValueRevision();
			~FlushChangedOptions() {
				if (agi::OptionValueRevision() == before) return;
				try {
					config::opt->Flush();
				}
				catch (agi::Exception const& error) {
					LOG_E("gpui/bridge") << "Could not save the configuration: " << error.GetMessage();
				}
			}
		} flush_changed_options;

		auto name_it = object.find("name");
		if (name_it == object.end())
			return R"({"ok":false,"error":"missing command name"})";
		std::string const& name = name_it->second;
		std::string_view const surface = gpui::CommandSurface(name);
		if (surface == "internal" || surface == "wx-control")
			return R"({"ok":false,"error":"this command still requires the original Neo Aegisub control"})";
		if (surface == "automation") {
			auto macros = config::global_scripts->GetMacros();
			auto local = context->local_scripts->GetMacros();
			macros.insert(macros.end(), local.begin(), local.end());
			if (std::none_of(macros.begin(), macros.end(), [&](auto macro) { return name == macro->name(); }))
				return R"({"ok":false,"error":"macro is not loaded for this project"})";
		}
		auto command = cmd::get(name);
		if (!command->Validate(context))
			return R"({"ok":false,"error":"command is not available in the current project state"})";
		if (name.starts_with("video/tool/")) { visual.SelectCommand(name); return R"({"ok":true})"; }
		if (auto selection = object.find("text_selection"); selection != object.end()) {
			auto const& range = static_cast<json::Array const&>(selection->second);
			auto line = context->selectionController->GetActiveLine();
			if (range.size() != 2 || !line) return R"({"ok":false,"error":"no active text selection"})";
			int64_t const start = static_cast<json::Integer const&>(range[0]);
			int64_t const end = static_cast<json::Integer const&>(range[1]);
			auto const& text = line->Text.get();
			auto boundary = [&](int64_t offset) {
				return offset >= 0 && static_cast<size_t>(offset) <= text.size() &&
					(static_cast<size_t>(offset) == text.size() || (static_cast<unsigned char>(text[offset]) & 0xc0) != 0x80);
			};
			if (start > end || !boundary(start) || !boundary(end)) return R"({"ok":false,"error":"invalid UTF-8 text selection"})";
			context->textSelectionController->SetSelection(start, end);
		}
		if (name == "edit/line/copy") { cmd::CopySelectedLines(context); return R"({"ok":true})"; }
		if (name == "edit/line/cut") { cmd::CutSelectedLines(context); return R"({"ok":true})"; }
		if (name == "edit/line/paste") { cmd::PasteLines(context); return R"({"ok":true})"; }
		if (name == "subtitle/open" || name == "subtitle/open/charset" || name == "subtitle/open/autosave" || name.starts_with("recent/subtitle/")) {
			if (context->subsController->TryToClose() == -1)
				return R"({"ok":false,"error":"opening subtitles was cancelled"})";
			agi::fs::path path;
			std::string encoding;
			if (name.starts_with("recent/subtitle/")) {
				int const index = std::stoi(name.substr(16));
				if (index < 0 || static_cast<size_t>(index) >= config::mru->Get("Subtitle")->size())
					return R"({"ok":false,"error":"recent subtitle is no longer available"})";
				path = config::mru->GetEntry("Subtitle", index);
			}
			else {
				// The front end owns the file picker and the encoding chooser;
				// it calls back into the `open` operation once the user picks.
				gpui::PushCommand(name);
				return R"({"ok":true})";
			}
			if (path.empty()) return R"({"ok":false,"error":"opening subtitles was cancelled"})";
			context->project->LoadSubtitles(path, encoding);
			return R"({"ok":true})";
		}
		if (name == "subtitle/new") {
			if (context->subsController->TryToClose() == -1)
				return R"({"ok":false,"error":"new subtitles was cancelled"})";
			context->project->CloseSubtitles();
			return R"({"ok":true})";
		}
		if (name == "video/play") {
			context->videoController->Play();
			return R"({"ok":true})";
		}
		if (name == "video/stop") {
			context->videoController->Stop();
			return R"({"ok":true})";
		}
		cmd::call(name, context);
		return R"({"ok":true})";
	}
};

GpuiBridge::GpuiBridge(agi::Context *context) : impl(std::make_unique<Impl>(context)) { }
GpuiBridge::~GpuiBridge() = default;

bool GpuiBridge::Start() {
	if (impl->server.Running()) return true;
	unsigned short port = 0;
	if (!impl->server.Listen(port)) {
		LOG_E("gpui/bridge") << "Could not bind the local GPUI bridge socket";
		return false;
	}
	impl->port = port;
	return true;
}

void GpuiBridge::Stop() { impl->Stop(); }
gpui::SocketServer &GpuiBridge::Server() { return impl->server; }
bool GpuiBridge::IsRunning() const { return impl->server.Running(); }
std::string GpuiBridge::Endpoint() const { return "127.0.0.1:" + std::to_string(impl->port); }
std::string GpuiBridge::Token() const { return impl->token; }
