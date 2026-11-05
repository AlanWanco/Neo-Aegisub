// Copyright (c) 2014, Thomas Goyne <plorkyeran@aegisub.org>
// Copyright (c) 2026, arch1t3cht
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

#include "video_properties.h"

#include "ass_file.h"
#include "async_video_provider.h"
#include "compat.h"
#include "format.h"
#include "gpui_bridge.h"
#include "options.h"
#include "resolution_resampler.h"

#include <libaegisub/color.h>

#include <cmath>
#include <optional>
#include <string>
#include <vector>

namespace ycbcr = agi::ycbcr;

namespace {

// This may look a bit silly with there being five copies of almost the same enum,
// (as well as various copies of very similar-looking code below), but the different
// mismatch cases are intentionally kept separate here so it's easier to reason about them
// and to adjust individual cases separately.

enum class ResolutionMismatchAction {
	Ignore,
	Prompt,
	Resample,
};

enum class ResolutionMismatchFix {
	Ignore,
	Resample,
};


enum class MissingLayoutResAction {
	Ignore,
	Prompt,
	Set,
	SetToPlayRes,
};

enum class MissingLayoutResFix {
	Ignore,
	Set,
	SetToPlayRes,
};


enum class MissingYCbCrMatrixAction {
	Ignore,
	Prompt,
	Set,
};

enum class MissingYCbCrMatrixFix {
	Ignore,
	Set,
};


enum class LayoutResMismatchPromptCondition {
	Never,
	OnARMismatch,
	Always,
};

enum class LayoutResMismatchFix {
	Ignore,
	Set,
};


enum class YCbCrMatrixMismatchAction {
	Ignore,
	Prompt,
	Set,
};

enum class YCbCrMatrixMismatchFix {
	Ignore,
	Set,
};


bool check_ar_changed(int sx, int sy, int vx, int vy) {
	if (sy == 0 || vy == 0)
		// This should be checked by the caller, but check it here for safety to avoid a division by zero crash.
		return (sy == 0) != (vy == 0);

	auto sar = double(sx) / sy;
	auto var = double(vx) / vy;
	return std::abs(sar - var) / var > .01;
}

/// Show a question whose answers are the given buttons. The answer arrives on
/// the front end, so `apply` carries the work and the synchronous return is
/// "cancelled": the caller keeps going with the script unchanged.
int prompt_choice_dialog(std::string const& message, std::string const& caption, std::vector<std::string> choices, int initial_selection, std::string const& suppress = {}, std::function<void(int)> apply = {}) {
	(void)initial_selection;
	gpui::PushChoice(caption, message, suppress, "choice", std::move(choices), std::move(apply));
	return -1;
}

bool update_ycbcr_matrix(AssFile *file, const AsyncVideoProvider *new_provider) {
	// When opening dummy video only want to set the script properties if
	// they were previously unset
	if (!new_provider->ShouldSetVideoProperties()) {
		return false;
	}

	auto VideoCS = new_provider->GetRealColorSpace();
	auto [guessCM, guessCR] = VideoCS;
	ycbcr::guess_colorspace(guessCM, guessCR, new_provider->GetWidth(), new_provider->GetHeight());

	if (VideoCS.matrix == agi::ycbcr_matrix::Unspecified && OPT_GET("Video/Untagged Matrix Warning")->GetBool()) {
		// Warn on an untagged matrix but not on an untagged range:
		// No sane player will ever guess a full range, so an untagged range is not really an issue in practice.

		auto untagged = fmt_tl(
			/* TRANSLATORS: Keep the space between the two line breaks; it's required for the message
			                to display correctly on Windows. */
			"The video you have loaded has no specified color matrix. "
			"Neo Aegisub will guess the color matrix to be %s, but there is no guarantee that other programs will guess the same matrix. "
			"This may make the video appear with different colors in different media players and can prevent subtitle colors from matching video colors."
			"\n \n"
			"Consider tagging your video with a color matrix to ensure that your video displays consistently in all players and that subtitle colors can reliably match video colors."
			, ycbcr::matrix_to_string(guessCM));
		gpui::PushNotice(from_wx(_("Untagged video")), from_wx(untagged));
	}

	if (new_provider->IsHDRorWCG() && OPT_GET("Video/HDR Video Warning")->GetBool()) {
		auto hdr = _(
			"The video you have loaded has HDR and/or WCG colors. "
			"While ordinary dialogue subtitles will work fine on such videos, "
			"matching video colors using subtitles will not be possible on HDR and/or WCG footage."
			);
		gpui::PushNotice(from_wx(_("HDR and/or WCG video")), from_wx(hdr));
	}

	auto script_matrix = file->GetYCbCrMatrix();
	auto recommended_matrix = ycbcr::Header(guessCM, guessCR).to_best_practice();

	if (std::holds_alternative<ycbcr::header_missing>(script_matrix) || std::holds_alternative<ycbcr::header_invalid>(script_matrix)) {
		// Script has no proper matrix set yet, so find out if the user wants to set one
		auto action = static_cast<MissingYCbCrMatrixAction>(OPT_GET("Video/No YCbCr Matrix in Script")->GetInt());

		if (action == MissingYCbCrMatrixAction::Prompt) {
			auto apply_choice = [file, recommended_matrix](int choice) {
				if (choice < 0) return;
				OPT_SET("Video/Last No YCbCr Matrix in Script Choice")->SetInt(choice);
				if (choice == static_cast<int>(MissingYCbCrMatrixFix::Set)) {
					file->SetScriptInfo("YCbCr Matrix", recommended_matrix.to_string().value());
					file->Commit(_("change ycbcr matrix"), AssFile::COMMIT_SCRIPTINFO);
				}
			};
			int Choice = prompt_choice_dialog(
				from_wx(fmt_tl(
					"The current subtitle file has no YCbCr Matrix set.\n\n"
					"If you plan to use this subtitle file on this newly loaded video file,\nyou should set the subtitle file's YCbCr Matrix to the video's color space (\"%s\").\n\n"
					"If you plan to use a different video file, you may want to\nleave the YCbCr Matrix unset until you load the correct video file.\n\n"
					"This prompt can be configured in the preferences.",
				recommended_matrix.to_string().value())),
				from_wx(_("Set script's YCbCr Matrix?")),
				{from_wx(_("Leave YCbCr Matrix unset")), from_wx(fmt_tl("Set script's YCbCr Matrix to the video's (\"%s\")", recommended_matrix.to_string().value()))},
				OPT_GET("Video/Last No YCbCr Matrix in Script Choice")->GetInt(),
				"Video/No YCbCr Matrix in Script",
				apply_choice
			);

			if (Choice == -1) {
				Choice = static_cast<int>(MissingYCbCrMatrixFix::Ignore);
			} else {
				OPT_SET("Video/Last No YCbCr Matrix in Script Choice")->SetInt(Choice);
			}

			switch (static_cast<MissingYCbCrMatrixFix>(Choice)) {
				case MissingYCbCrMatrixFix::Ignore:
					action = MissingYCbCrMatrixAction::Ignore;
					break;
				case MissingYCbCrMatrixFix::Set:
					action = MissingYCbCrMatrixAction::Set;
					break;
			}
		}

		if (action == MissingYCbCrMatrixAction::Set) {
			file->SetScriptInfo("YCbCr Matrix", recommended_matrix.to_string().value());
			return true;
		}
	} else if (std::holds_alternative<ycbcr::header_colorspace>(script_matrix) && script_matrix != recommended_matrix) {
		// If the script's YCbCr Matrix is None, we assume that the user opts out of all color mangling
		// and do not attempt (or ask) to change the YCbCr Matrix further.
		//
		// If the script's YCbCr Matrix is some actual color space, see if the video's color space is different from it.
		auto action = static_cast<YCbCrMatrixMismatchAction>(OPT_GET("Video/YCbCr Matrix Mismatch")->GetInt());

		// This may be the most complicated case of all the video property mismatches.
		// There are the following relevant cases:
		// 1. The opened video is not a video on which the subtitles are meant to be played back. (Say, it's some test video or the user selected the wrong video by mistake.)
		//	  This is the easy case, there the user can just leave the script unchanged.
		// 2. The script was originally authored on (say) a BT.601 encode of some RGB video, and the user is now loading a (say) BT.709 encode of the same RGB video.
		//	  In this case, the RGB colors in the subtitle file remain accurate to the new video (decoded with its tagged matrix), so the correct action is to set the new YCbCr Matrix header without changing colors.
		// 3. The video the script was initially authored for was actually (say) BT.601 but mistagged as (say) BT.709 (or untagged and wrongly guessed),
		//	  and the user is now loading the same YCbCr stream, but this time correctly tagged as BT.601.
		//    In this case, the RGB colors in the subtitle file are *not* accurate to the video, so the correct action is to either leave the YCbCr Matrix unchanged or resample the colors to the new YCbCr Matrix.
		//
		//    We imagine that this case is the rarer one. (At least when exclusively working with new files, where Aegisub will also no longer force BT.601 and will warn on untagged videos.
		//    Old, mistagged subtitle files are a different story.)
		//
		// (In all of these cases the base assumption is that the script looked correct on whatever video it was originally authored on.
		// If this is not the case, all bets are off and the user will need to manually fix the matrix in the script properties dialog.)

		// FIXME: Also offer a "resample" option here. The added difficulty here is that not all color spaces can be resampled to (see dialog_resample.cpp).
		//		  As explained above, in the cases where this is necessary the YCbCr Matrix can also be left unchanged, so it's left as a lower priority for now.

		if (action == YCbCrMatrixMismatchAction::Prompt) {
			auto apply_choice = [file, recommended_matrix](int choice) {
				if (choice < 0) return;
				OPT_SET("Video/Last YCbCr Matrix Mismatch Choice")->SetInt(choice);
				if (choice == static_cast<int>(YCbCrMatrixMismatchFix::Set)) {
					file->SetScriptInfo("YCbCr Matrix", recommended_matrix.to_string().value());
					file->Commit(_("change ycbcr matrix"), AssFile::COMMIT_SCRIPTINFO);
				}
			};
			int Choice = prompt_choice_dialog(
				from_wx(fmt_tl(
					"This video's recommended YCbCr Matrix (\"%s\") differs from the subtitle file's YCbCr Matrix (\"%s\").\n\n"
					"If this new video's colors look identical to the video the subtitle file was originally authored for,\n"
					"then the subtitle file's YCbCr Matrix should be set to the video's YCbCr Matrix.",
				recommended_matrix.to_string().value(), script_matrix.to_string().value())),
				from_wx(_("YCbCr Matrix mismatch")),
				{from_wx(fmt_tl("Leave script's YCbCr Matrix as it is (\"%s\")", script_matrix.to_string().value())), from_wx(fmt_tl("Set script's YCbCr Matrix to the video's (\"%s\")", recommended_matrix.to_string().value()))},
				OPT_GET("Video/Last YCbCr Matrix Mismatch Choice")->GetInt(),
				{},
				apply_choice
			);

			if (Choice == -1) {
				Choice = static_cast<int>(YCbCrMatrixMismatchFix::Ignore);
			} else {
				OPT_SET("Video/Last YCbCr Matrix Mismatch Choice")->SetInt(Choice);
			}

			switch (static_cast<YCbCrMatrixMismatchFix>(Choice)) {
				case YCbCrMatrixMismatchFix::Ignore:
					action = YCbCrMatrixMismatchAction::Ignore;
					break;
				case YCbCrMatrixMismatchFix::Set:
					action = YCbCrMatrixMismatchAction::Set;
					break;
			}
		}

		if (action == YCbCrMatrixMismatchAction::Set) {
			file->SetScriptInfo("YCbCr Matrix", recommended_matrix.to_string().value());
			return true;
		}
	}

	return false;
}

bool update_play_res(AssFile *file, const AsyncVideoProvider *new_provider) {
	// Check that the script resolution matches the video resolution
	int sx = file->GetScriptInfoAsInt("PlayResX");
	int sy = file->GetScriptInfoAsInt("PlayResY");

	auto [vx, vy] = new_provider->GetDisplayResolution();

	// If the script resolution hasn't been set at all just force it to the
	// video resolution
	if (sx == 0 && sy == 0) {
		file->SetScriptInfo("PlayResX", std::to_string(vx));
		file->SetScriptInfo("PlayResY", std::to_string(vy));
		return true;
	}

	// When opening dummy video only want to set the script properties if
	// they were previously unset
	if (!new_provider->ShouldSetVideoProperties())
		return false;

	// Treat exact multiples of the video resolution as equaling the resolution
	// for the people who use that for subpixel precision (which is mostly
	// pointless these days due to decimals being supported almost everywhere)
	if (sx % vx == 0 && sy % vy == 0)
		return false;

	bool ar_changed = check_ar_changed(sx, sy, vx, vy);

	switch (static_cast<ResolutionMismatchAction>(OPT_GET("Video/PlayRes Mismatch")->GetInt())) {
	case ResolutionMismatchAction::Ignore: default:
		return false;

	case ResolutionMismatchAction::Resample:
		// Fallthrough to prompt if the AR changed
		if (!ar_changed) {
			ResampleResolution(file, {
				{0, 0, 0, 0},
				sx, sy, vx, vy,
				ResampleARMode::Stretch,
				std::nullopt,
			});
			return true;
		}
		[[fallthrough]];

	case ResolutionMismatchAction::Prompt:
		std::vector<std::string> choices;
		if (ar_changed) {
			choices = {
				from_wx(_("Leave resolution as it is")),
				from_wx(_("Resample script (stretch to new aspect ratio)")),
				from_wx(_("Resample script (add borders)")),
				from_wx(_("Resample script (remove borders)"))
			};
		} else {
			choices = {
				from_wx(_("Leave resolution as it is")),
				from_wx(_("Resample script")),
			};
		}

		int res = prompt_choice_dialog(
			from_wx(fmt_tl(
				"The resolution of the loaded video and the resolution specified for the subtitles don't match.\n\n"
				"Video resolution:\t%d \u00D7 %d\n"     // U+00D7 multiplication sign
				"Script resolution:\t%d \u00D7 %d\n\n"
				"Change subtitles resolution to match video?", vx, vy, sx, sy)),
			from_wx(_("Resolution mismatch")),
			std::move(choices),
			OPT_GET("Video/Last PlayRes Mismatch Choice")->GetInt()
		);

		if (res == -1) {
			res = static_cast<int>(ResolutionMismatchFix::Ignore);
		} else {
			OPT_SET("Video/Last PlayRes Mismatch Choice")->SetInt(res);
		}

		auto action = static_cast<ResolutionMismatchFix>(res);

		if (action == ResolutionMismatchFix::Ignore) return false;

		ResampleResolution(file, {
			{0, 0, 0, 0},
			sx, sy, vx, vy,
			static_cast<ResampleARMode>(static_cast<int>(action) - static_cast<int>(ResolutionMismatchFix::Resample)),
			std::nullopt,
		});
		return true;
	}
}

bool update_layout_res(AssFile *file, const AsyncVideoProvider *new_provider) {
	// When opening dummy video only want to set the script properties if
	// they were previously unset
	if (!new_provider->ShouldSetVideoProperties())
		return false;

	int spx = file->GetScriptInfoAsInt("PlayResX");
	int spy = file->GetScriptInfoAsInt("PlayResY");

	int sx = file->GetScriptInfoAsInt("LayoutResX");
	int sy = file->GetScriptInfoAsInt("LayoutResY");

	auto [vx, vy] = new_provider->GetDisplayResolution();

	if (sx == 0 || sy == 0) {
		// Script has no LayoutRes set yet, so find out if the user wants to set one
		auto action = static_cast<MissingLayoutResAction>(OPT_GET("Video/No LayoutRes in Script")->GetInt());

		if (action == MissingLayoutResAction::Prompt) {
			auto apply_choice = [file, vx, vy, spx, spy](int choice) {
				if (choice < 0) return;
				OPT_SET("Video/Last No LayoutRes in Script Choice")->SetInt(choice);
				if (choice == static_cast<int>(MissingLayoutResFix::Set)) {
					file->SetScriptInfo("LayoutResX", std::to_string(vx));
					file->SetScriptInfo("LayoutResY", std::to_string(vy));
					file->Commit(_("change layout resolution"), AssFile::COMMIT_SCRIPTINFO);
				} else if (choice == static_cast<int>(MissingLayoutResFix::SetToPlayRes)) {
					file->SetScriptInfo("LayoutResX", std::to_string(spx));
					file->SetScriptInfo("LayoutResY", std::to_string(spy));
					file->Commit(_("change layout resolution"), AssFile::COMMIT_SCRIPTINFO);
				}
			};
			int Choice = prompt_choice_dialog(
				from_wx(fmt_tl(
					"The current subtitle file has no layout resolution set.\n\n"
					"If you plan to use this subtitle file on this newly loaded video file,\n"
					"you should set the subtitle file's layout resolution to the video's display resolution.\n"
					"\n"
					"If you plan to use a different video file, you may want to leave\n"
					"the layout resolution unset until you load the correct video file.\n"
					"\n"
					"If you are adapting an existing old subtitle file that does not yet have\n"
					"a layout resolution set, you should set its layout resolution to the resolution\n"
					"of the video the subtitle file was intended to be played back on.\n"
					"Often, this is the same as the file's script resolution. (%d \u00D7 %d)\n"
					"\n"
					"This prompt can be configured in the preferences."
				, spx, spy)),
				from_wx(_("Set script's layout resolution?")),
				{from_wx(_("Leave layout resolution unset")),
				 from_wx(fmt_tl("Set script's layout resolution to the video's (%d \u00D7 %d)", vx, vy)), // U+00D7 multiplication sign
				 from_wx(fmt_tl("Set script's layout resolution to its script resolution (%d \u00D7 %d)", spx, spy))},
				OPT_GET("Video/Last No LayoutRes in Script Choice")->GetInt(),
				"Video/No LayoutRes in Script",
				apply_choice
			);

			if (Choice == -1) {
				Choice = static_cast<int>(MissingLayoutResFix::Ignore);
			} else {
				OPT_SET("Video/Last No LayoutRes in Script Choice")->SetInt(Choice);
			}

			switch (static_cast<MissingLayoutResFix>(Choice)) {
				case MissingLayoutResFix::Ignore:
					action = MissingLayoutResAction::Ignore;
					break;
				case MissingLayoutResFix::Set:
					action = MissingLayoutResAction::Set;
					break;
				case MissingLayoutResFix::SetToPlayRes:
					action = MissingLayoutResAction::SetToPlayRes;
					break;
			}
		}

		switch (action) {
			case MissingLayoutResAction::Set:
				file->SetScriptInfo("LayoutResX", std::to_string(vx));
				file->SetScriptInfo("LayoutResY", std::to_string(vy));
				return true;
			case MissingLayoutResAction::SetToPlayRes:
				file->SetScriptInfo("LayoutResX", std::to_string(spx));
				file->SetScriptInfo("LayoutResY", std::to_string(spy));
				return true;
			default:
				break;
		}
	} else if (sx != vx || sy != vy) {
		// The video's (display) resolution differs from the LayoutRes.
		//
		// If the aspect ratio stays the same this is usually not a problem
		// (after all, the entire point of LayoutRes is that subtitle files will correctly display
		// on all video resolutions), so we do not show any alert here by default.
		// However, if the user wants to be prompted in these cases, they can enable this in the settings.
		//
		// If the aspect ratio changes, we definitely need user intervention here.

		auto condition = static_cast<LayoutResMismatchPromptCondition>(OPT_GET("Video/LayoutRes Mismatch")->GetInt());

		if (condition == LayoutResMismatchPromptCondition::Never)
			return false;

		bool ar_changed = check_ar_changed(sx, sy, vx, vy);

		if (!ar_changed && condition == LayoutResMismatchPromptCondition::OnARMismatch)
			return false;

		auto prompt = !ar_changed ?
			fmt_tl(
				"The resolution of the loaded video and the layout resolution specified for the subtitles don't match.\n\n"
				"Video resolution:    %d \u00D7 %d\n"     // U+00D7 multiplication sign
				"Subtitle layout resolution:    %d \u00D7 %d\n\n"
				"Usually, this is not an issue, and you may leave the subtitle file's layout resolution as it is.\n"
				"If your subtitle file does not contain any significant formatting and you now intend to perform\n"
				"formatting on the newly loaded video, you may want to set the script's layout resolution\n"
				"to the video's display resolution."
			, vx, vy, sx, sy)
			:
			// FIXME: offer more options in this case, like cropping or resampling to the new aspect ratio
			fmt_tl(
				"The resolution of the loaded video and the layout resolution specified for the subtitles differ in aspect ratios.\n\n"
				"Video resolution:    %d \u00D7 %d\n"     // U+00D7 multiplication sign
				"Subtitle layout resolution:    %d \u00D7 %d\n\n"
				"Change layout resolution of subtitles to match video?"
			, vx, vy, sx, sy);

		auto apply_choice = [file, vx, vy](int choice) {
			if (choice < 0) return;
			OPT_SET("Video/Last LayoutRes Mismatch Choice")->SetInt(choice);
			if (static_cast<LayoutResMismatchFix>(choice) == LayoutResMismatchFix::Set) {
				file->SetScriptInfo("LayoutResX", std::to_string(vx));
				file->SetScriptInfo("LayoutResY", std::to_string(vy));
				file->Commit(_("change layout resolution"), AssFile::COMMIT_SCRIPTINFO);
			}
		};
		int Choice = prompt_choice_dialog(
			from_wx(prompt),
			from_wx(_("Layout resolution mismatch")),
			{from_wx(fmt_tl("Leave layout resolution as it is (%d \u00D7 %d)", sx, sy)), from_wx(fmt_tl("Set script's layout resolution to the video's (%d \u00D7 %d)", vx, vy))}, // U+00D7 multiplication sign
			OPT_GET("Video/Last LayoutRes Mismatch Choice")->GetInt(),
			{},
			apply_choice
		);

		if (Choice == -1) {
			Choice = static_cast<int>(LayoutResMismatchFix::Ignore);
		} else {
			OPT_SET("Video/Last LayoutRes Mismatch Choice")->SetInt(Choice);
		}

		if (static_cast<LayoutResMismatchFix>(Choice) == LayoutResMismatchFix::Set) {
			file->SetScriptInfo("LayoutResX", std::to_string(vx));
			file->SetScriptInfo("LayoutResY", std::to_string(vy));
			return true;
		}
	}

	return false;
}

}

void UpdateVideoProperties(AssFile *file, const AsyncVideoProvider *new_provider) {
	if (update_ycbcr_matrix(file, new_provider))
		file->Commit(_("change ycbcr matrix"), AssFile::COMMIT_SCRIPTINFO);

	if (update_play_res(file, new_provider))
		file->Commit(_("change script resolution"), AssFile::COMMIT_SCRIPTINFO);

	if (update_layout_res(file, new_provider))
		file->Commit(_("change layout resolution"), AssFile::COMMIT_SCRIPTINFO);
}
