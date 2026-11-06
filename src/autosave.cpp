// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

/// @file autosave.cpp
/// @brief Enumerate the autosave, backup and recovered files the host offers
/// @ingroup files

#include "autosave.h"
#include "localisation.h"

#include "options.h"

#include <libaegisub/path.h>

#include <algorithm>
#include <chrono>
#include <cstdio>
#include <ctime>
#include <filesystem>
#include <map>

namespace {
/// Seconds carried by a `%Y-%m-%d-%H-%M-%S` stamp in local time, or 0 when the
/// stamp is missing or out of range. Sscanf keeps this portable: Windows has no
/// strptime.
int64_t stamp_seconds(std::string const& value) {
	int year = 0, month = 0, day = 0, hour = 0, minute = 0, second = 0;
	if (std::sscanf(value.c_str(), "%d-%d-%d-%d-%d-%d", &year, &month, &day, &hour, &minute, &second) != 6)
		return 0;
	if (year < 1970 || month < 1 || month > 12 || day < 1 || day > 31 || hour < 0 || hour > 23 || minute < 0 || minute > 59 || second < 0 || second > 60)
		return 0;

	std::tm tm {};
	tm.tm_year = year - 1900;
	tm.tm_mon = month - 1;
	tm.tm_mday = day;
	tm.tm_hour = hour;
	tm.tm_min = minute;
	tm.tm_sec = second;
	tm.tm_isdst = -1;
	std::time_t stamp = std::mktime(&tm);
	return stamp == static_cast<std::time_t>(-1) ? 0 : static_cast<int64_t>(stamp);
}

std::string stamp_display(int64_t seconds) {
	std::time_t stamp = static_cast<std::time_t>(seconds);
	std::tm local {};
#ifdef _WIN32
	if (localtime_s(&local, &stamp)) return {};
#else
	if (!localtime_r(&stamp, &local)) return {};
#endif
	char buffer[64] {};
	if (!std::strftime(buffer, sizeof buffer, "%c", &local)) return {};
	return buffer;
}

int64_t modification_seconds(std::filesystem::path const& file) {
	std::error_code error;
	auto stamp = std::filesystem::last_write_time(file, error);
	if (error) return 0;
	auto since_epoch = std::chrono::time_point_cast<std::chrono::system_clock::duration>(
		stamp - std::filesystem::file_time_type::clock::now() + std::chrono::system_clock::now());
	return std::chrono::duration_cast<std::chrono::seconds>(since_epoch.time_since_epoch()).count();
}

void populate(std::map<std::string, AutosaveFile>& files, std::string const& path_option,
              std::string const& filter, std::string const& suffix, std::string const& kind) {
	std::string directory = config::path->Decode(path_option).string();
	std::error_code error;
	if (!std::filesystem::is_directory(directory, error)) return;

	for (auto const& entry : std::filesystem::directory_iterator(directory, error)) {
		if (error) break;
		std::string name = entry.path().filename().string();
		if (name.size() <= filter.size() || name.compare(name.size() - filter.size(), filter.size(), filter) != 0)
			continue;
		if (!entry.is_regular_file(error)) continue;

		std::string stem = name.substr(0, name.size() - filter.size());
		std::string stamp = stem.substr(stem.rfind('.') == std::string::npos ? stem.size() : stem.rfind('.') + 1);
		std::string base = stem.rfind('.') == std::string::npos ? std::string() : stem.substr(0, stem.rfind('.'));
		int64_t timestamp = stamp_seconds(stamp);
		if (base.empty())
			base = stamp;
		else if (!timestamp)
			base += "." + stamp;

		AutosaveFile& file = files[base];
		file.name = base;
		file.versions.push_back({
			entry.path().string(),
			stamp_display(timestamp ? timestamp : modification_seconds(entry.path())) + suffix,
			kind,
			timestamp ? timestamp : modification_seconds(entry.path()),
		});
	}
}
}

std::vector<AutosaveFile> GetAutosaveFiles() {
	std::map<std::string, AutosaveFile> files;
	populate(files, "Path/Auto/Save", ".AUTOSAVE.ass", "", "autosave");
	populate(files, "Path/Auto/Backup", ".ORIGINAL.ass", _(" [ORIGINAL BACKUP]"), "backup");
	populate(files, "?user/recovered", ".ass", _(" [RECOVERED]"), "recovered");

	std::vector<AutosaveFile> result;
	for (auto& [name, file] : files) {
		std::stable_sort(file.versions.begin(), file.versions.end(),
			[](AutosaveVersion const& a, AutosaveVersion const& b) { return a.timestamp > b.timestamp; });
		result.push_back(std::move(file));
	}
	std::stable_sort(result.begin(), result.end(), [](AutosaveFile const& a, AutosaveFile const& b) {
		return a.versions[0].timestamp > b.versions[0].timestamp;
	});
	return result;
}
