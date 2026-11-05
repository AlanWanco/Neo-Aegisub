// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

#pragma once

#include <libaegisub/cajun/elements.h>
#include <memory>
#include <string>
#include <string_view>

namespace agi { struct Context; }
namespace gpui {
class Visual {
	struct Impl;
	std::unique_ptr<Impl> impl;
public:
	explicit Visual(agi::Context *context);
	~Visual();
	std::string Interact(json::Object const& request);
	bool IsActive(std::string_view command) const;
	void SelectCommand(std::string_view command);
};
}
