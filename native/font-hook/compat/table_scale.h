#pragma once
#include <string>

// Artemis 表中的绝对字号来自不同窗口；不能把正文、姓名和注音统一成一个值。
// 只改完整的正整数常量，保留表达式、注释和普通字符串。调用方每次提供原始表。
namespace GalGearTable {
inline bool Ident(char c) {
    return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') ||
        (c >= '0' && c <= '9') || c == '_';
}
inline size_t QuotedEnd(const std::string& text, size_t pos) {
    char quote = text[pos++];
    while (pos < text.size()) {
        if (text[pos] == '\\' && pos + 1 < text.size()) pos += 2;
        else if (text[pos++] == quote) return pos;
    }
    return pos;
}
inline bool Scale(std::string& text, unsigned percent) {
    if (percent < 50 || percent > 300 || percent == 100) return false;
    bool changed = false;
    for (size_t pos = 0; pos < text.size();) {
        if (text.compare(pos, 2, "--") == 0) {
            if (text.compare(pos + 2, 2, "[[") == 0) {
                auto end = text.find("]]", pos + 4);
                pos = end == std::string::npos ? text.size() : end + 2;
            } else {
                auto end = text.find_first_of("\r\n", pos);
                pos = end == std::string::npos ? text.size() : end;
            }
            continue;
        }
        size_t start = pos;
        std::string key;
        if (text[pos] == '\'' || text[pos] == '"') {
            pos = QuotedEnd(text, pos);
            if (pos > start + 1) key = text.substr(start + 1, pos - start - 2);
        } else if (Ident(text[pos])) {
            while (pos < text.size() && Ident(text[pos])) ++pos;
            key = text.substr(start, pos - start);
        } else { ++pos; continue; }
        if (key != "size" && key != "rubysize") continue;
        size_t p = pos;
        while (p < text.size() && (text[p] == ' ' || text[p] == '\t')) ++p;
        if (p == text.size() || (text[p] != '=' && text[p] != ':')) continue;
        ++p;
        while (p < text.size() && (text[p] == ' ' || text[p] == '\t')) ++p;
        char quote = 0;
        if (p < text.size() && (text[p] == '\'' || text[p] == '"')) quote = text[p++];
        size_t valueStart = p;
        unsigned value = 0;
        while (p < text.size() && text[p] >= '0' && text[p] <= '9') {
            if (value <= 4096) value = value * 10 + (text[p] - '0');
            ++p;
        }
        if (p == valueStart || value == 0 || value > 4096) continue;
        size_t end = p;
        if (quote) {
            if (p == text.size() || text[p] != quote) continue;
            ++end;
        }
        while (end < text.size() && (text[end] == ' ' || text[end] == '\t')) ++end;
        if (end < text.size() && text[end] != ',' && text[end] != '}' &&
            text[end] != ';' && text[end] != '\r' && text[end] != '\n' &&
            text.compare(end, 2, "--") != 0) continue;
        auto replacement = std::to_string((value * percent + 50) / 100);
        text.replace(valueStart, p - valueStart, replacement);
        pos = valueStart + replacement.size() + (quote ? 1 : 0);
        changed = true;
    }
    return changed;
}
}
