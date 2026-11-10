// Copyright (c) 2006, 2007, Niels Martin Hansen
// All rights reserved.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are met:
//
//   * Redistributions of source code must retain the above copyright notice,
//     this list of conditions and the following disclaimer.
//   * Redistributions in binary form must reproduce the above copyright notice,
//     this list of conditions and the following disclaimer in the documentation
//     and/or other materials provided with the distribution.
//   * Neither the name of the Aegisub Group nor the names of its contributors
//     may be used to endorse or promote products derived from this software
//     without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
// AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
// IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
// ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE
// LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
// CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
// SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
// INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
// CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
// ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
// POSSIBILITY OF SUCH DAMAGE.
//
// Aegisub Project http://www.aegisub.org/

/// @file auto4_lua_progresssink.cpp
/// @brief Lua 5.1-based scripting engine
/// @ingroup scripting
///

#include "auto4_lua.h"

#include "gpui_bridge.h"

#include <libaegisub/cajun/elements.h>
#include <libaegisub/color.h>
#include <libaegisub/lua/utils.h>

#include <algorithm>
#include <cctype>
#include <charconv>
#include <cfloat>
#include <climits>
#include <cmath>
#include <cstdint>
#include <iterator>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

using namespace agi::lua;

namespace {
    template<lua_CFunction fn>
    void set_field_to_closure(lua_State *L, const char *name, int ps_idx = -3)
    {
        lua_pushvalue(L, ps_idx);
        lua_pushcclosure(L, exception_wrapper<fn>, 1);
        lua_setfield(L, -2, name);
    }

    void set_field_to_nil(lua_State *L, int idx, const char *name)
    {
        lua_pushnil(L);
        lua_setfield(L, idx, name);
    }

    using json::UnknownElement;

    // Control classes, defaults and clamping follow upstream auto4_lua_dialog.cpp.
    // Unlike upstream, malformed fields and unknown classes raise Lua errors instead of being ignored.

    [[noreturn]] void field_error(lua_State *L, char const *control, char const *field, char const *type)
    {
        error(L, "control '%s': field '%s' must be a %s", control, field, type);
    }

    std::string string_field(lua_State *L, char const *control, char const *field, std::string def)
    {
        lua_getfield(L, -1, field);
        if (!lua_isnil(L, -1)) {
            if (!lua_isstring(L, -1))
                field_error(L, control, field, "string");
            size_t len = 0;
            char const *str = lua_tolstring(L, -1, &len);
            def.assign(str, len);
        }
        lua_pop(L, 1);
        return def;
    }

    std::optional<double> number_field(lua_State *L, char const *control, char const *field)
    {
        lua_getfield(L, -1, field);
        std::optional<double> value;
        if (!lua_isnil(L, -1)) {
            if (!lua_isnumber(L, -1))
                field_error(L, control, field, "number");
            double const number = lua_tonumber(L, -1);
            if (!std::isfinite(number)) {
                // JSON has no infinities; unlimited script bounds use the finite endpoints.
                if (std::isinf(number) && (std::string_view(field) == "min" || std::string_view(field) == "max"))
                    value = std::copysign(DBL_MAX, number);
                else
                    field_error(L, control, field, "finite number");
            }
            else value = number;
        }
        lua_pop(L, 1);
        return value;
    }

    std::optional<int64_t> int_field(lua_State *L, char const *control, char const *field)
    {
        auto number = number_field(L, control, field);
        if (!number)
            return std::nullopt;
        if (*number < INT_MIN || *number > INT_MAX)
            error(L, "control '%s': field '%s' is out of range", control, field);
        return static_cast<int64_t>(*number);
    }

    std::optional<bool> bool_field(lua_State *L, char const *control, char const *field)
    {
        lua_getfield(L, -1, field);
        std::optional<bool> value;
        if (!lua_isnil(L, -1)) {
            if (!lua_isboolean(L, -1))
                field_error(L, control, field, "boolean");
            value = lua_toboolean(L, -1) != 0;
        }
        lua_pop(L, 1);
        return value;
    }

    json::Array string_array_field(lua_State *L, char const *control, char const *field)
    {
        json::Array items;
        lua_getfield(L, -1, field);
        if (!lua_isnil(L, -1)) {
            if (!lua_istable(L, -1))
                field_error(L, control, field, "array of strings");
            // Read by index so the order is the order written in the script.
            for (int i = 1;; ++i) {
                lua_rawgeti(L, -1, i);
                if (lua_isnil(L, -1)) {
                    lua_pop(L, 1);
                    break;
                }
                if (!lua_isstring(L, -1))
                    field_error(L, control, field, "array of strings");
                size_t len = 0;
                char const *str = lua_tolstring(L, -1, &len);
                items.emplace_back(json::String(str, len));
                lua_pop(L, 1);
            }
        }
        lua_pop(L, 1);
        return items;
    }

    template<class T>
    T clamp_to(T value, T lo, T hi)
    {
        return std::min(std::max(value, lo), hi);
    }

    bool is_button_id(std::string const& id)
    {
        static char const *const ids[] = {"ok", "yes", "save", "apply", "close", "no", "cancel", "help", "context_help"};
        return std::find(std::begin(ids), std::end(ids), id) != std::end(ids);
    }

    agi::Color dialog_color(lua_State *L, std::string_view text)
    {
        if (text.size() == 9 && text.front() == '#') {
            unsigned rgba = 0;
            auto [end, err] = std::from_chars(text.data() + 1, text.data() + text.size(), rgba, 16);
            if (err != std::errc{} || end != text.data() + text.size())
                error(L, "invalid hexadecimal colour");
            return agi::Color(rgba >> 24, rgba >> 16, rgba >> 8, rgba);
        }
        return agi::Color(text);
    }

    // Parses the control table at the top of the stack into its normalized form.
    json::Object parse_control(lua_State *L)
    {
        std::string cls = string_field(L, "control", "class", "");
        std::transform(cls.begin(), cls.end(), cls.begin(), [](unsigned char c) { return static_cast<char>(std::tolower(c)); });
        if (cls.empty())
            error(L, "bad control table entry");
        char const *control = cls.c_str();

        json::Object object;
        object["class"] = json::String(cls);
        object["name"] = json::String(string_field(L, control, "name", ""));
        object["hint"] = json::String(string_field(L, control, "hint", ""));
        object["x"] = json::Integer(int_field(L, control, "x").value_or(0));
        object["y"] = json::Integer(int_field(L, control, "y").value_or(0));
        object["width"] = json::Integer(int_field(L, control, "width").value_or(1));
        object["height"] = json::Integer(int_field(L, control, "height").value_or(1));

        if (cls == "label") {
            object["label"] = json::String(string_field(L, control, "label", ""));
        }
        else if (cls == "edit" || cls == "textbox" || cls == "alpha") {
            // As upstream: 'text' is also accepted and overrides 'value'.
            std::string const text = string_field(L, control, "value", "");
            object["value"] = json::String(string_field(L, control, "text", text));
        }
        else if (cls == "intedit") {
            int64_t lo = int_field(L, control, "min").value_or(INT_MIN);
            int64_t hi = int_field(L, control, "max").value_or(INT_MAX);
            if (lo >= hi) {
                lo = INT_MIN;
                hi = INT_MAX;
            }
            int64_t const value = clamp_to<int64_t>(int_field(L, control, "value").value_or(0), lo, hi);
            object["value"] = json::Integer(value);
            object["min"] = json::Integer(lo);
            object["max"] = json::Integer(hi);
        }
        else if (cls == "floatedit") {
            double lo = number_field(L, control, "min").value_or(-DBL_MAX);
            double hi = number_field(L, control, "max").value_or(DBL_MAX);
            double const step = number_field(L, control, "step").value_or(0.0);
            double value = number_field(L, control, "value").value_or(0.0);
            if (lo >= hi) {
                lo = -DBL_MAX;
                hi = DBL_MAX;
            }
            if (step != 0.0) {
                if (lo == -DBL_MAX)
                    lo = 0.0;
                if (hi == DBL_MAX)
                    hi = 100.0;
            }
            value = clamp_to(value, lo, hi);
            object["value"] = json::Double(value);
            object["min"] = json::Double(lo);
            object["max"] = json::Double(hi);
            object["step"] = json::Double(step);
        }
        else if (cls == "dropdown") {
            object["value"] = json::String(string_field(L, control, "value", ""));
            object["items"] = string_array_field(L, control, "items");
        }
        else if (cls == "checkbox") {
            object["label"] = json::String(string_field(L, control, "label", ""));
            object["value"] = json::Boolean(bool_field(L, control, "value").value_or(false));
        }
        else if (cls == "color" || cls == "coloralpha") {
            std::string const text = string_field(L, control, "value", "");
            auto const color = dialog_color(L, text);
            object["value"] = json::String(color.GetHexFormatted(cls == "coloralpha"));
        }
        else {
            error(L, "bad control table entry: unknown class '%s'", control);
        }

        return object;
    }

    // Typed access to a result element; wrong types are Lua errors.
    template<class T>
    T const& typed(lua_State *L, UnknownElement const& element, char const *name)
    {
        try {
            return static_cast<T const&>(element);
        }
        catch (json::Exception const&) {
            error(L, "bad value for control '%s'", name);
        }
    }

    double number_value(lua_State *L, UnknownElement const& element, char const *name)
    {
        try {
            return static_cast<json::Double const&>(element);
        }
        catch (json::Exception const&) {
            return typed<json::Integer>(L, element, name);
        }
    }

    // Pushes one control's value normalized the way the wx control would have produced it.
    void push_control_value(lua_State *L, json::Object const& control, char const *name, UnknownElement const& value)
    {
        std::string const cls = static_cast<json::String const&>(control.at("class"));
        if (cls == "edit" || cls == "textbox" || cls == "alpha" || cls == "dropdown") {
            push_value(L, typed<json::String>(L, value, name));
        }
        else if (cls == "color" || cls == "coloralpha") {
            std::string const text = typed<json::String>(L, value, name);
            auto const color = dialog_color(L, text);
            push_value(L, color.GetHexFormatted(cls == "coloralpha"));
        }
        else if (cls == "intedit") {
            int64_t const lo = static_cast<json::Integer const&>(control.at("min"));
            int64_t const hi = static_cast<json::Integer const&>(control.at("max"));
            push_value(L, clamp_to<int64_t>(typed<json::Integer>(L, value, name), lo, hi));
        }
        else if (cls == "floatedit") {
            double const lo = static_cast<json::Double const&>(control.at("min"));
            double const hi = static_cast<json::Double const&>(control.at("max"));
            push_value(L, clamp_to(number_value(L, value, name), lo, hi));
        }
        else if (cls == "checkbox") {
            push_value(L, typed<json::Boolean>(L, value, name));
        }
        else {
            error(L, "bad control table entry: unknown class '%s'", cls.c_str());
        }
    }

    // Pushes the picker result: nil on cancel, a string, or an array of strings when multiple.
    int push_picked_paths(lua_State *L, json::Object const& answer, bool multiple)
    {
        std::vector<std::string> paths;
        if (auto found = answer.find("paths"); found != answer.end()) {
            for (auto const& path : static_cast<json::Array const&>(found->second))
                paths.push_back(static_cast<json::String const&>(path));
        }

        if (paths.empty()) {
            lua_pushnil(L);
        }
        else if (multiple) {
            lua_createtable(L, paths.size(), 0);
            for (size_t i = 0; i < paths.size(); ++i) {
                push_value(L, paths[i]);
                lua_rawseti(L, -2, i + 1);
            }
        }
        else {
            push_value(L, paths.front());
        }
        return 1;
    }
}

namespace Automation4 {
    json::Object LuaDialogDefinition(lua_State *L, int controls, int buttons, int button_ids)
    {
        // Resolve negative indices now, before anything is pushed.
        int const top = lua_gettop(L);
        auto resolve = [top](int idx) { return idx < 0 ? top + idx + 1 : idx; };
        controls = resolve(controls);
        buttons = resolve(buttons);
        button_ids = resolve(button_ids);

        if (!lua_istable(L, controls))
            error(L, "Cannot create config dialog from something non-table");

        json::Array control_list;
        for (int i = 1;; ++i) {
            lua_rawgeti(L, controls, i);
            if (lua_isnil(L, -1)) {
                lua_pop(L, 1);
                break;
            }
            if (!lua_istable(L, -1))
                error(L, "bad control table entry");
            control_list.emplace_back(parse_control(L));
            lua_pop(L, 1);
        }

        json::Array button_list;
        if (buttons) {
            if (!lua_istable(L, buttons))
                error(L, "buttons must be a table");
            for (int i = 1;; ++i) {
                lua_rawgeti(L, buttons, i);
                if (lua_isnil(L, -1)) {
                    lua_pop(L, 1);
                    break;
                }
                json::Object button;
                button["label"] = json::String(check_string(L, -1));
                button["id"] = json::String("");
                button_list.emplace_back(std::move(button));
                lua_pop(L, 1);
            }
        }

        if (button_ids) {
            if (!lua_istable(L, button_ids))
                error(L, "button_ids must be a table");
            lua_pushnil(L);
            while (lua_next(L, button_ids)) {
                if (lua_type(L, -2) != LUA_TSTRING)
                    error(L, "button id must be a string");
                std::string const id = lua_tostring(L, -2);
                std::string const label = check_string(L, -1);
                if (!is_button_id(id))
                    error(L, "Invalid button id '%s'", id.c_str());
                auto match = std::find_if(button_list.begin(), button_list.end(), [&](UnknownElement const& element) {
                    auto const& button = static_cast<json::Object const&>(element);
                    return static_cast<json::String const&>(button.at("label")) == label
                        && static_cast<json::String const&>(button.at("id")).empty();
                });
                if (match == button_list.end())
                    error(L, "Invalid button for id %s", id.c_str());
                static_cast<json::Object&>(*match)["id"] = json::String(id);
                lua_pop(L, 1);
            }
        }

        json::Object definition;
        definition["controls"] = std::move(control_list);
        definition["buttons"] = std::move(button_list);
        return definition;
    }

    void LuaDialogPushValues(lua_State *L, json::Object const& definition, json::Object const& values)
    {
        lua_createtable(L, 0, 0);
        for (auto const& entry : static_cast<json::Array const&>(definition.at("controls"))) {
            auto const& control = static_cast<json::Object const&>(entry);
            std::string const cls = static_cast<json::String const&>(control.at("class"));
            if (cls == "label")
                continue;
            std::string const name = static_cast<json::String const&>(control.at("name"));
            auto found = values.find(name);
            UnknownElement const& value = found == values.end() ? control.at("value") : found->second;
            push_control_value(L, control, name.c_str(), value);
            lua_setfield(L, -2, name.c_str());
        }
    }

    int LuaProgressSink::LuaDisplayDialog(lua_State *L)
    {
        int const buttons = lua_isnoneornil(L, 2) ? 0 : 2;
        int const button_ids = lua_isnoneornil(L, 3) ? 0 : 3;

        json::Object definition = LuaDialogDefinition(L, 1, buttons, button_ids);
        definition["kind"] = json::String("form");
        definition["title"] = json::String("");

        json::Object const answer = gpui::AskScriptDialog(definition);

        // Missing button (no front end, or cancelled) means cancel.
        int64_t button = -1;
        if (auto found = answer.find("button"); found != answer.end())
            button = static_cast<json::Integer const&>(found->second);

        json::Object const no_values;
        auto found_values = answer.find("values");
        json::Object const& values = found_values == answer.end()
            ? no_values
            : static_cast<json::Object const&>(found_values->second);

        auto const& labels = static_cast<json::Array const&>(definition.at("buttons"));
        if (button == -1) {
            lua_pushboolean(L, false);
        }
        else if (labels.empty()) {
            // Backend default pair: 0 is OK (true), 1 is Cancel (false).
            if (button != 0 && button != 1)
                error(L, "script dialog answered unknown button %d", static_cast<int>(button));
            lua_pushboolean(L, button == 0);
        }
        else {
            if (button < 0 || static_cast<size_t>(button) >= labels.size())
                error(L, "script dialog answered unknown button %d", static_cast<int>(button));
            auto const& choice = static_cast<json::Object const&>(labels[static_cast<size_t>(button)]);
            if (static_cast<json::String const&>(choice.at("id")) == "cancel")
                lua_pushboolean(L, false);
            else
                push_value(L, static_cast<json::String const&>(choice.at("label")));
        }

        LuaDialogPushValues(L, definition, values);
        return 2;
    }

    int LuaProgressSink::LuaDisplayOpenDialog(lua_State *L)
    {
        bool const multiple = lua_toboolean(L, 5) != 0;
        bool const must_exist = lua_isnoneornil(L, 6) || lua_toboolean(L, 6) != 0;

        json::Object request;
        request["kind"] = json::String("open");
        request["title"] = json::String(check_string(L, 1));
        request["directory"] = json::String(check_string(L, 2));
        request["filename"] = json::String(check_string(L, 3));
        request["wildcard"] = json::String(check_string(L, 4));
        request["multiple"] = json::Boolean(multiple);
        request["must_exist"] = json::Boolean(must_exist);
        request["no_overwrite_prompt"] = json::Boolean(false);

        json::Object const answer = gpui::AskScriptDialog(request);
        return push_picked_paths(L, answer, multiple);
    }

    int LuaProgressSink::LuaDisplaySaveDialog(lua_State *L)
    {
        json::Object request;
        request["kind"] = json::String("save");
        request["title"] = json::String(check_string(L, 1));
        request["directory"] = json::String(check_string(L, 2));
        request["filename"] = json::String(check_string(L, 3));
        request["wildcard"] = json::String(check_string(L, 4));
        request["multiple"] = json::Boolean(false);
        request["must_exist"] = json::Boolean(false);
        request["no_overwrite_prompt"] = json::Boolean(lua_toboolean(L, 5) != 0);

        json::Object const answer = gpui::AskScriptDialog(request);
        return push_picked_paths(L, answer, false);
    }

    LuaProgressSink::LuaProgressSink(lua_State *L, ProgressSink *ps, bool allow_config_dialog)
    : L(L)
    {
        auto ud = (ProgressSink**)lua_newuserdata(L, sizeof(ProgressSink*));
        *ud = ps;

        // register progress reporting stuff
        lua_getglobal(L, "aegisub");

        // Create aegisub.progress table
        lua_createtable(L, 0, 5);
        set_field_to_closure<LuaSetProgress>(L, "set");
        set_field_to_closure<LuaSetTask>(L, "task");
        set_field_to_closure<LuaSetTitle>(L, "title");
        set_field_to_closure<LuaGetCancelled>(L, "is_cancelled");
        lua_setfield(L, -2, "progress");

        // Create aegisub.debug table
        lua_createtable(L, 0, 4);
        set_field_to_closure<LuaDebugOut>(L, "out");
        lua_setfield(L, -2, "debug");

        // Set aegisub.log
        set_field_to_closure<LuaDebugOut>(L, "log", -2);

        if (allow_config_dialog) {
            lua_createtable(L, 0, 3);
            set_field_to_closure<LuaDisplayDialog>(L, "display");
            set_field_to_closure<LuaDisplayOpenDialog>(L, "open");
            set_field_to_closure<LuaDisplaySaveDialog>(L, "save");
            lua_setfield(L, -2, "dialog");
        }

        // reference so other objects can also find the progress sink
        lua_pushvalue(L, -2);
        lua_setfield(L, LUA_REGISTRYINDEX, "progress_sink");

        lua_pop(L, 2);
    }

    LuaProgressSink::~LuaProgressSink()
    {
        // remove progress reporting stuff
        lua_getglobal(L, "aegisub");
        set_field_to_nil(L, -2, "progress");
        set_field_to_nil(L, -2, "debug");
        lua_pop(L, 1);

        set_field_to_nil(L, LUA_REGISTRYINDEX, "progress_sink");
    }

    ProgressSink* LuaProgressSink::GetObjPointer(lua_State *L, int idx)
    {
        assert(lua_type(L, idx) == LUA_TUSERDATA);
        return *((ProgressSink**)lua_touserdata(L, idx));
    }

    int LuaProgressSink::LuaSetProgress(lua_State *L)
    {
        GetObjPointer(L, lua_upvalueindex(1))->SetProgress(lua_tonumber(L, 1), 100);
        return 0;
    }

    int LuaProgressSink::LuaSetTask(lua_State *L)
    {
        GetObjPointer(L, lua_upvalueindex(1))->SetMessage(check_string(L, 1));
        return 0;
    }

    int LuaProgressSink::LuaSetTitle(lua_State *L)
    {
        GetObjPointer(L, lua_upvalueindex(1))->SetTitle(check_string(L, 1));
        return 0;
    }

    int LuaProgressSink::LuaGetCancelled(lua_State *L)
    {
        lua_pushboolean(L, GetObjPointer(L, lua_upvalueindex(1))->IsCancelled());
        return 1;
    }

    int LuaProgressSink::LuaDebugOut(lua_State *L)
    {
        ProgressSink *ps = GetObjPointer(L, lua_upvalueindex(1));

        // Check trace level
        if (lua_type(L, 1) == LUA_TNUMBER) {
            if (lua_tointeger(L, 1) > ps->GetTraceLevel())
                return 0;
            // remove trace level
            lua_remove(L, 1);
        }

        // Only do format-string handling if there's more than one argument left
        // (If there's more than one argument left, assume first is a format string and rest are format arguments)
        if (lua_gettop(L) > 1) {
            // Format the string
            lua_getglobal(L, "string");
            lua_getfield(L, -1, "format");
            // Here stack contains format string, format arguments, 'string' table, format function
            // remove 'string' table
            lua_remove(L, -2);
            // put the format function into place
            lua_insert(L, 1);
            // call format function
            if (lua_pcall(L, lua_gettop(L) - 1, 1, 0)) {
                // format failed so top of the stack now has an error message
                // which we want to add position information to
                luaL_where(L, 1);
                lua_insert(L, 1);
                lua_concat(L, 2);
                throw error_tag{};
            }
        }

        // Top of stack is now a string to output
        ps->Log(check_string(L, 1));
        return 0;
    }
}
