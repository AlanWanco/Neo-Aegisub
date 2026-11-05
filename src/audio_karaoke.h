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

#pragma once

#include <libaegisub/signal.h>

#include <memory>
#include <string_view>

class AssDialogue;
namespace agi { class AudioProvider; }
namespace agi { struct Context; }
namespace agi::ass { class Karaoke; }

/// @class AudioKaraoke
/// @brief Karaoke mode controller
///
/// Owns the shared Karaoke model for the active line and switches the audio
/// controller between the dialogue and karaoke timing controllers. The split
/// and join interface itself is drawn by the front end, which drives this
/// class through `EditSplit`, `CommitSplit`, `RevertSplit` and `ChangeTagType`.
class AudioKaraoke final : private agi::signal::ConnectionScope {
	agi::Context *c; ///< Project context
	agi::signal::Connection file_changed; ///< File changed slot
	agi::signal::Connection audio_opened; ///< Audio opened connection
	agi::signal::Connection active_line_changed;

	/// Currently active dialogue line
	AssDialogue *active_line = nullptr;

	/// Karaoke data
	std::unique_ptr<agi::ass::Karaoke> kara;

	bool enabled = false; ///< Is karaoke mode enabled?

	/// Load syllable data from the currently active line
	void LoadFromLine();
	/// Reset all pending split information and return to normal mode
	void CancelSplit();
	/// Apply any pending split information to the syllable data and return to normal mode
	void AcceptSplit();
	/// Set the karaoke tags for the selected syllables to the indicated one
	void SetTagType(std::string_view new_type);

	void OnActiveLineChanged(AssDialogue *new_line);
	void OnFileChanged(int type, const AssDialogue *changed);
	void OnAudioOpened(agi::AudioProvider *provider);

public:
	/// @param c Project context
	explicit AudioKaraoke(agi::Context *c);
	~AudioKaraoke();

	/// Is karaoke mode currently enabled?
	bool IsEnabled() const { return enabled; }

	/// Enable or disable karaoke mode
	void SetEnabled(bool enable);

	/// Shared karaoke model and split workflow for other frontends.
	agi::ass::Karaoke const& GetModel() const { return *kara; }
	void EditSplit(size_t syllable, size_t position, bool remove);
	void CommitSplit() { AcceptSplit(); }
	void RevertSplit() { CancelSplit(); }
	void ChangeTagType(std::string_view tag) { SetTagType(tag); }
};
