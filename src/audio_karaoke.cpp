// Copyright (c) 2013, Thomas Goyne <plorkyeran@aegisub.org>
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

#include "audio_karaoke.h"

#include "include/aegisub/context.h"

#include "ass_dialogue.h"
#include "ass_file.h"
#include "ass_karaoke.h"
#include "audio_controller.h"
#include "audio_timing.h"
#include "format.h"
#include "project.h"
#include "selection_controller.h"

#include <libaegisub/ass/karaoke.h>
#include <libaegisub/log.h>

AudioKaraoke::AudioKaraoke(agi::Context *c)
: c(c)
, file_changed(c->ass->AddCommitListener(&AudioKaraoke::OnFileChanged, this))
, audio_opened(c->project->AddAudioProviderListener(&AudioKaraoke::OnAudioOpened, this))
, active_line_changed(c->selectionController->AddActiveLineListener(&AudioKaraoke::OnActiveLineChanged, this))
, kara(std::make_unique<agi::ass::Karaoke>())
{
}

AudioKaraoke::~AudioKaraoke() {
}

void AudioKaraoke::OnActiveLineChanged(AssDialogue *new_line) {
	active_line = new_line;
	if (enabled) LoadFromLine();
}

void AudioKaraoke::OnFileChanged(int type, const AssDialogue *changed) {
	if (enabled && (type & AssFile::COMMIT_DIAG_FULL) && (!changed || changed == active_line))
		LoadFromLine();
}

void AudioKaraoke::OnAudioOpened(agi::AudioProvider *provider) {
	if (provider) {
		if (enabled) SetEnabled(false);
	}
	else {
		enabled = false;
		c->audioController->SetTimingController(nullptr);
	}
}

void AudioKaraoke::SetEnabled(bool en) {
	if (en == enabled) return;
	if (en && !active_line) {
		LOG_D("karaoke") << "no active line to time";
		return;
	}
	if (en && !c->project->AudioProvider()) {
		LOG_D("karaoke") << "no audio to time";
		return;
	}

	enabled = en;
	if (enabled) {
		LoadFromLine();
		c->audioController->SetTimingController(CreateKaraokeTimingController(c, kara.get(), file_changed));
	}
	else {
		CancelSplit();
		c->audioController->SetTimingController(CreateDialogueTimingController(c));
	}
}

void AudioKaraoke::EditSplit(size_t syllable, size_t position, bool remove) {
	if (!enabled || !active_line) return;
	if (remove) kara->RemoveSplit(syllable);
	else kara->AddSplit(syllable, position);
}

void AudioKaraoke::SetTagType(std::string_view new_tag) {
	kara->SetTagType(new_tag);
	AcceptSplit();
}

void AudioKaraoke::LoadFromLine() {
	if (!active_line) return;
	SetKaraokeLine(*kara, active_line, true);
}

void AudioKaraoke::CancelSplit() {
	LoadFromLine();
}

void AudioKaraoke::AcceptSplit() {
	if (!active_line) return;
	active_line->Text = kara->GetText();
	file_changed.Block();
	c->ass->Commit(_("karaoke split"), AssFile::COMMIT_DIAG_TEXT);
	file_changed.Unblock();
}
