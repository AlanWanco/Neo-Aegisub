i18n = import('i18n')
# This should only be used to compile the translations during a build.
# Meson's i18n module is not sufficient to generate a pot file:
# It misses translatable strings not directly found in either
# C++ source, desktop or appdata file. This affects strings
# of the Windows installer (iss), from Lua scripts and JSON files.
# Until a solution is found, POT updates should continue to use make_pot.sh.
# Similarly, since meson insists on regenerating the POT file before updating
# PO files, PO files should be updated using update_po.sh
i18n.gettext('aegisub',
             args: [
                '-k_', '-kSTR_MENU', '-kSTR_DISP', '-kSTR_HELP', '-kCOMMAND_GROUP:5,',
                '-kfmt_tl', '-kfmt_plural:2,3',
                '--sort-by-file'
             ],
             install_dir: localedir)

# `i18n.gettext` only compiles catalogs while installing, so a plain build
# tree has no .mo files and every wx string falls back to English. This
# target writes <build>/po/<lang>/LC_MESSAGES/aegisub.mo for launchers that
# export AEGISUB_LOCALE_DIR, so a build-tree run matches an installed build.
run_target('catalogs',
           command: [find_program('build_catalogs.sh'),
                     meson.current_build_dir(),
                     meson.current_source_dir()])
