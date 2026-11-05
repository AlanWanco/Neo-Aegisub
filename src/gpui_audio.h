// Copyright (c) 2026, Aegisub Project
// SPDX-License-Identifier: BSD-3-Clause

#pragma once

#include <libaegisub/cajun/elements.h>
#include <cstdint>
#include <memory>
#include <string>
#include <vector>

namespace agi { struct Context; }
namespace gpui {
/// Reply whose `head` is one JSON object (no trailing newline) followed on the
/// wire by '\n' and then the raw `body` bytes. `body` is empty for JSON-only
/// replies.
struct RawReply {
    std::string head;
    std::vector<unsigned char> body;
};

class Audio {
    struct Impl;
    std::unique_ptr<Impl> impl;
public:
    explicit Audio(agi::Context *context);
    ~Audio();
    std::string Snapshot();
    RawReply Render(json::Object const& request);
    std::string Interact(json::Object const& request);
};
}
