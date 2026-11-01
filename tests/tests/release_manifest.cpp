// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

#include "../../src/release_manifest.h"

#include <gtest/gtest.h>

namespace {
json::Object Release(std::string tag, std::string asset, bool draft = false) {
	json::Object result;
	result["name"] = tag;
	result["tag_name"] = tag;
	result["draft"] = draft;
	result["prerelease"] = true;
	result["html_url"] = std::string(updates::releases_url) + "/tag/" + tag;
	result["body"] = "Release notes";
	json::Object package;
	package["name"] = std::move(asset);
	json::Array assets;
	assets.emplace_back(std::move(package));
	result["assets"] = std::move(assets);
	return result;
}
}

TEST(ReleaseManifest, PreviewRevisionsIgnoreApiOrder) {
	// The fork API returns these preview builds in publication order.
	json::Array releases;
	releases.emplace_back(Release("preview-9928", "Aegisub-9928-macOS-arm64.dmg"));
	releases.emplace_back(Release("preview-9931", "Aegisub-9931-Windows-x64.zip"));
	releases.emplace_back(Release("preview-9925", "Aegisub-9925-macOS-arm64.dmg"));
	auto update = updates::FindUpdate(releases, 9927, {3, 4, 2});
	ASSERT_TRUE(update);
	EXPECT_EQ(9931, update->revision);
	EXPECT_EQ("preview-9931", update->name);
	EXPECT_EQ("https://github.com/AlanWanco/Aegisub/releases/tag/preview-9931", update->url);
	EXPECT_FALSE(updates::FindUpdate(releases, 9931, {3, 4, 2}));
	EXPECT_FALSE(updates::FindUpdate(releases, 10000, {3, 4, 2}));
}

TEST(ReleaseManifest, SkipsDraftAndSourceOnlyReleases) {
	json::Array releases;
	releases.emplace_back(Release("preview-9999", "Aegisub-9999-macOS-arm64.dmg", true));
	releases.emplace_back(Release("preview-9998", "Aegisub-9998-source.tar.gz"));
	releases.emplace_back(Release("preview-9997", "Aegisub-9997-macOS-arm64.dmg.sha256"));
	releases.emplace_back(Release("preview-9931", "Aegisub-9931-macOS-arm64.dmg"));
	auto update = updates::FindUpdate(releases, 9928, {3, 4, 2});
	ASSERT_TRUE(update);
	EXPECT_EQ(9931, update->revision);
	json::Array source_only;
	source_only.emplace_back(Release("preview-9999", "source.zip"));
	EXPECT_FALSE(updates::FindUpdate(source_only, 9928, {3, 4, 2}));
}

TEST(ReleaseManifest, ComparesOfficialVersionsNumerically) {
	json::Array releases;
	releases.emplace_back(Release("v3.9.0", "Aegisub-3.9.0-macOS.dmg"));
	releases.emplace_back(Release("v3.10.0", "Aegisub-3.10.0-macOS.dmg"));
	releases.emplace_back(Release("v3.8.0", "Aegisub-99999-Windows.zip"));
	auto update = updates::FindUpdate(releases, 10000, {3, 9, 0});
	ASSERT_TRUE(update);
	EXPECT_EQ("v3.10.0", update->name);
	EXPECT_FALSE(updates::FindUpdate(releases, 10000, {3, 10, 0}));
	EXPECT_FALSE(updates::FindUpdate(releases, 10000, {4, 0, 0}));
}

TEST(ReleaseManifest, AllowsNullBody) {
	auto release = Release("preview-9931", "Aegisub-9931-macOS-arm64.dmg");
	release["body"] = json::Null{};
	json::Array releases;
	releases.emplace_back(std::move(release));
	auto update = updates::FindUpdate(releases, 9928, {3, 4, 2});
	ASSERT_TRUE(update);
	EXPECT_TRUE(update->description.empty());
}

TEST(ReleaseManifest, RejectsMalformedManifest) {
	json::Array missing_fields;
	missing_fields.emplace_back(json::Object{});
	EXPECT_THROW(updates::FindUpdate(missing_fields, 0, {3, 4, 2}), std::out_of_range);
	json::Array wrong_release_type;
	wrong_release_type.emplace_back("not a release");
	EXPECT_THROW(updates::FindUpdate(wrong_release_type, 0, {3, 4, 2}), json::Exception);
	auto release = Release("preview-9931", "Aegisub-9931-macOS-arm64.dmg");
	release["assets"] = "not an array";
	json::Array wrong_assets_type;
	wrong_assets_type.emplace_back(std::move(release));
	EXPECT_THROW(updates::FindUpdate(wrong_assets_type, 0, {3, 4, 2}), json::Exception);
	release = Release("preview-9931", "Aegisub-9931-macOS-arm64.dmg");
	release["body"] = true;
	json::Array wrong_body_type;
	wrong_body_type.emplace_back(std::move(release));
	EXPECT_THROW(updates::FindUpdate(wrong_body_type, 0, {3, 4, 2}), json::Exception);
}
