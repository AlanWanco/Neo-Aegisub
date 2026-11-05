subdir('libresrc')

gpui_host_src = files(
    'MatroskaParser.c',
    'aegisublocale.cpp',
    'ass_attachment.cpp',
    'ass_dialogue.cpp',
    'ass_entry.cpp',
    'ass_export_filter.cpp',
    'ass_exporter.cpp',
    'ass_file.cpp',
    'ass_karaoke.cpp',
    'ass_override.cpp',
    'ass_parser.cpp',
    'ass_style.cpp',
    'ass_style_storage.cpp',
    'async_video_provider.cpp',
    'audio_colorscheme.cpp',
    'audio_controller.cpp',
    'audio_marker.cpp',
    'audio_player.cpp',
    'audio_provider_factory.cpp',
    'audio_renderer.cpp',
    'audio_renderer_spectrum.cpp',
    'audio_renderer_waveform.cpp',
    'audio_karaoke.cpp',
    'audio_timing_dialogue.cpp',
    'audio_timing_karaoke.cpp',
    'autosave.cpp',
    'auto4_base.cpp',
    'auto4_lua.cpp',
    'auto4_lua_assfile.cpp',
    'auto4_lua_progresssink.cpp',
    'charset_detect.cpp',
    'colorspace.cpp',
    'command/app.cpp',
    'command/audio.cpp',
    'command/automation.cpp',
    'command/command.cpp',
    'command/edit.cpp',
    'command/grid.cpp',
    'command/help.cpp',
    'command/keyframe.cpp',
    'command/recent.cpp',
    'command/subtitle.cpp',
    'command/time.cpp',
    'command/timecode.cpp',
    'command/tool.cpp',
    'command/video.cpp',
    'command/vis_tool.cpp',
    'compat.cpp',
    'context.cpp',
    'ebu_export_settings.cpp',
    'export_fixstyle.cpp',
    'export_framerate.cpp',
    'fft.cpp',
    'font_file_lister.cpp',
    'gpui_audio.cpp',
    'gpui_automation.cpp',
    'gpui_bridge.cpp',
    'gpui_loop.cpp',
    'gpui_color.cpp',
    'gpui_command_catalog.cpp',
    'gpui_dialogs.cpp',
    'gpui_document.cpp',
    'grid_display_text.cpp',
    'gpui_translation.cpp',
    'gpui_timing.cpp',
    'gpui_search.cpp',
    'gpui_server.cpp',
    'gpui_visual.cpp',
    'gpui_video.cpp',
    'gpui_preferences.cpp',
    'gpui_spelling.cpp',
    'hotkey.cpp',
    'initial_line_state.cpp',
    'menu.cpp',
    'mkv_wrap.cpp',
    'pen.cpp',
    'project.cpp',
    'progress_runner.cpp',
    'provider_file_formats.cpp',
    'resolution_resampler.cpp',
    'search_replace_engine.cpp',
    'selection_controller.cpp',
    'shift_times.cpp',
    'spellchecker.cpp',
    'spline.cpp',
    'spline_curve.cpp',
    'subs_controller.cpp',
    'subtitle_format.cpp',
    'subtitle_format_ass.cpp',
    'subtitle_format_ebu3264.cpp',
    'subtitle_format_encore.cpp',
    'subtitle_format_microdvd.cpp',
    'subtitle_format_srt.cpp',
    'subtitle_format_ssa.cpp',
    'subtitle_format_transtation.cpp',
    'subtitle_format_ttxt.cpp',
    'subtitle_format_txt.cpp',
    'subtitles_provider.cpp',
    'subtitles_provider_libass.cpp',
    'text_file_reader.cpp',
    'text_file_writer.cpp',
    'text_selection_controller.cpp',
    'thesaurus.cpp',
    'timing_processor.cpp',
    'utils.cpp',
    'vector2d.cpp',
    'version.cpp',
    'version_check.cpp',
    'video_controller.cpp',
    'video_properties.cpp',
    'video_frame.cpp',
    'video_provider_cache.cpp',
    'video_provider_dummy.cpp',
    'video_provider_manager.cpp',
    'video_provider_yuv4mpeg.cpp',
    'visual_feature.cpp',
    'visual_render.cpp',
    'gl_text.cpp',
    'gl_wrap.cpp',
    'visual_tool.cpp',
    'visual_tool_clip.cpp',
    'visual_tool_cross.cpp',
    'visual_tool_drag.cpp',
    'visual_tool_rotatexy.cpp',
    'visual_tool_rotatez.cpp',
    'visual_tool_scale.cpp',
    'visual_tool_vector_clip.cpp',
)


if host_machine.system() == 'darwin'
    gpui_host_src += files(
        'font_file_lister_coretext.mm',
        'osx/osx_utils.mm',
    )

elif host_machine.system() == 'windows'
    gpui_host_src += files(
        'avisynth_wrap.cpp',
        'crash_writer_minidump.cpp',
        'font_file_lister_gdi.cpp',
        'video_provider_avs.cpp',
    )

    if cc.has_header('wingdi.h')
        deps += cc.find_library('gdi32', required: true)
    else
        error('Missing Windows SDK GDI Library (wingdi.h / gdi32.lib)')
    endif
    if cc.has_header('dwrite.h')
        deps += cc.find_library('dwrite', required: true)
    else
        error('Missing Windows SDK DirectWrite Library (dwrite.h / dwrite.lib)')
    endif

endif

if host_machine.system() != 'windows'
    gpui_host_src += files('crash_writer.cpp')
endif

if conf.has('WITH_FONTCONFIG')
    gpui_host_src += files('font_file_lister_fontconfig.cpp')
endif

if conf.has('WITH_CSRI')
    gpui_host_src += files('subtitles_provider_csri.cpp')
endif

opt_src = [
    ['ALSA', 'audio_player_alsa.cpp'],
    ['PortAudio', 'audio_player_portaudio.cpp'],
    ['PulseAudio', 'audio_player_pulse.cpp'],
    ['OpenAL', 'audio_player_openal.cpp'],
    ['OSS', 'audio_player_oss.cpp'],
    ['DirectSound', ['audio_player_dsound.cpp',
                     'audio_player_dsound2.cpp']],

    ['FFMS2', ['audio_provider_ffmpegsource.cpp',
               'video_provider_ffmpegsource.cpp',
               'ffmpegsource_common.cpp']],

    ['Hunspell', 'spellchecker_hunspell.cpp'],

    ['libportal', 'xdg_desktop_portal_utils.cpp'],
]

foreach opt: opt_src
    if dep_avail.contains(opt[0])
        gpui_host_src += files(opt[1])
    endif
endforeach

