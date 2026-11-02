// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause
#pragma once

#include <libaegisub/cajun/elements.h>

#include <algorithm>
#include <array>
#include <charconv>
#include <optional>
#include <string>
#include <string_view>

namespace updates {
inline constexpr char releases_url[] = "https://github.com/AlanWanco/Aegisub/releases";

struct Release {
	std::string name;
	std::string url;
	std::string description;
	int revision = 0;
	std::optional<std::array<int, 3>> version;
};

inline std::optional<std::array<int, 3>> Version(std::string_view tag) {
	if (tag.starts_with('v')) tag.remove_prefix(1);
	std::array<int, 3> result{};
	for (size_t i = 0; i < result.size(); ++i) {
		auto [end, error] = std::from_chars(tag.data(), tag.data() + tag.size(), result[i]);
		if (error != std::errc{} || result[i] < 0) return {};
		tag.remove_prefix(end - tag.data());
		if (i + 1 < result.size()) {
			if (!tag.starts_with('.')) return {};
			tag.remove_prefix(1);
		}
	}
	return tag.empty() ? std::optional(result) : std::nullopt;
}

// The fork publishes preview tags and can republish older builds later. Package
// revision numbers, rather than API order or publication date, identify upgrades.
inline std::optional<Release> FindUpdate(json::Array const& releases, int revision,
                                        std::array<int, 3> version) {
	std::optional<Release> latest;
	for (auto const& value : releases) {
		auto const& item = static_cast<json::Object const&>(value);
		if (static_cast<json::Boolean const&>(item.at("draft"))) continue;
		Release release;
		release.name = static_cast<std::string const&>(item.at("name"));
		release.url = static_cast<std::string const&>(item.at("html_url"));
		if (!release.url.starts_with("https://github.com/AlanWanco/Aegisub/releases/tag/"))
			continue;
		if (auto body = item.find("body"); body != item.end()) {
			try { release.description = static_cast<std::string const&>(body->second); }
			catch (json::Exception const&) { static_cast<json::Null const&>(body->second); }
		}
		release.version = Version(static_cast<std::string const&>(item.at("tag_name")));
		bool packaged = false;
		for (auto const& asset_value : static_cast<json::Array const&>(item.at("assets"))) {
			auto const& asset = static_cast<json::Object const&>(asset_value);
			std::string_view name = static_cast<std::string const&>(asset.at("name"));
			if (name.find("source") != std::string_view::npos ||
			    !(name.ends_with(".dmg") || name.ends_with(".exe") || name.ends_with(".zip") ||
			      name.ends_with(".AppImage") || name.ends_with(".deb") || name.ends_with(".rpm") ||
			      name.ends_with(".tar.gz") || name.ends_with(".tar.xz"))) continue;
			if (!name.starts_with("Neo-Aegisub-")) continue;
			name.remove_prefix(8);
			int number = 0;
			auto [end, error] = std::from_chars(name.data(), name.data() + name.size(), number);
			if (error == std::errc{} && number > 0 && end != name.data() + name.size() && *end == '-') {
				release.revision = std::max(release.revision, number);
				packaged = true;
			}
			else if (release.version) packaged = true;
		}
		if (!packaged) continue;
		bool newer_version = release.version && *release.version > version;
		if (release.version && *release.version < version) continue;
		if (!newer_version && release.revision <= revision) continue;
		bool latest_version = latest && latest->version && *latest->version > version;
		if (!latest || (newer_version && !latest_version) ||
		    (newer_version && latest_version && *release.version > *latest->version) ||
		    (newer_version == latest_version && (!newer_version || release.version == latest->version) &&
		     release.revision > latest->revision))
			latest = std::move(release);
	}
	return latest;
}
}
