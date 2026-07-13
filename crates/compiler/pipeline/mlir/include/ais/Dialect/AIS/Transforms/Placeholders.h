/**
 * @file  Placeholders.h
 * @brief Shared helpers for parsing named `{name}` template placeholders
 *        and reading/writing positional LLM input contracts.
 *
 * Single source of truth for placeholder syntax across all AIS C++ passes.
 * Templates carry references like `{question}` or `{ctx0}`. The
 * `input_names` ArrayAttr enumerates the human-readable name of each Data
 * input in operand order. The optional `input_roles` ArrayAttr is positional
 * with it and classifies whether an input is user-renderable or protected
 * semantic context.
 *
 * Numeric `{0}` placeholders are NOT supported here: the Rust validator
 * rejects them upstream, so by the time MLIR sees a template, every
 * placeholder is a named identifier.
 */

#ifndef APXM_AIS_TRANSFORMS_PLACEHOLDERS_H
#define APXM_AIS_TRANSFORMS_PLACEHOLDERS_H

#include "ais/Common/Constants.h"

#include "mlir/IR/Builders.h"
#include "mlir/IR/BuiltinAttributes.h"
#include "mlir/IR/Operation.h"
#include "llvm/ADT/DenseMap.h"
#include "llvm/ADT/SmallVector.h"
#include "llvm/ADT/StringRef.h"
#include "llvm/ADT/StringSet.h"
#include "llvm/ADT/StringSwitch.h"

#include <string>

namespace mlir::ais::placeholders {

/// Semantic channel assigned to one LLM context operand.
enum class PromptInputRole {
  User,
  System,
  DependencyOnly,
  ToolContext,
  Control,
  Invalid,
};

// Mirror of `attrs::INPUT_ROLES` in apxm-ais.
inline constexpr llvm::StringLiteral kInputRolesAttrName = "input_roles";
inline constexpr llvm::StringLiteral kLegacySystemPromptInputName = "__system";

/// Parse one exact serialized input role.
inline PromptInputRole parseInputRole(llvm::StringRef role) {
  return llvm::StringSwitch<PromptInputRole>(role)
      .Case("user", PromptInputRole::User)
      .Case("system", PromptInputRole::System)
      .Case("dependency_only", PromptInputRole::DependencyOnly)
      .Case("tool_context", PromptInputRole::ToolContext)
      .Case("control", PromptInputRole::Control)
      .Default(PromptInputRole::Invalid);
}

/// Return the serialized spelling for a valid input role.
inline llvm::StringRef inputRoleName(PromptInputRole role) {
  switch (role) {
  case PromptInputRole::User:
    return "user";
  case PromptInputRole::System:
    return "system";
  case PromptInputRole::DependencyOnly:
    return "dependency_only";
  case PromptInputRole::ToolContext:
    return "tool_context";
  case PromptInputRole::Control:
    return "control";
  case PromptInputRole::Invalid:
    return {};
  }
  return {};
}

/// Return whether a role may be rendered as a user-template placeholder.
inline bool isUserRole(PromptInputRole role) {
  return role == PromptInputRole::User;
}

/// Normalize the legacy system input name only when no explicit role exists.
inline PromptInputRole roleForLegacyInputName(llvm::StringRef inputName) {
  // Mirror of `attrs::LEGACY_SYSTEM_PROMPT_INPUT_NAME` in apxm-ais.
  return inputName == kLegacySystemPromptInputName ? PromptInputRole::System
                                                    : PromptInputRole::User;
}

/// Walk a template string and append every `{name}` identifier (the bare
/// word inside the braces) to `out`, preserving order of first appearance
/// and skipping duplicates. Returns nothing — the caller owns `out`.
inline void collectNames(llvm::StringRef templateStr,
                         llvm::SmallVectorImpl<llvm::StringRef> &out) {
  llvm::StringSet<> seen;
  for (size_t i = 0, n = templateStr.size(); i < n; ++i) {
    if (templateStr[i] != '{')
      continue;
    if (i + 1 >= n)
      break;
    size_t close = templateStr.find('}', i + 1);
    if (close == llvm::StringRef::npos)
      break;
    llvm::StringRef name = templateStr.slice(i + 1, close);
    // Accept only `\w+` (word chars). Empty / non-word names are ignored.
    bool valid = !name.empty();
    for (char c : name) {
      if (!(std::isalnum(static_cast<unsigned char>(c)) || c == '_')) {
        valid = false;
        break;
      }
    }
    if (valid && seen.insert(name).second)
      out.push_back(name);
    i = close;
  }
}

/// Convenience: returns the names as an owning vector.
inline llvm::SmallVector<llvm::StringRef> namesIn(llvm::StringRef templateStr) {
  llvm::SmallVector<llvm::StringRef> result;
  collectNames(templateStr, result);
  return result;
}

/// Read the `input_names` ArrayAttr off `op` into a vector of StringRefs.
/// Returns an empty vector if the attribute is missing or malformed.
/// The returned StringRefs alias storage owned by the MLIR context, so they
/// remain valid for the lifetime of the op.
inline llvm::SmallVector<llvm::StringRef> readInputNames(mlir::Operation *op) {
  llvm::SmallVector<llvm::StringRef> result;
  auto attr = op->getAttrOfType<mlir::ArrayAttr>(
      apxm::constants::attrs::INPUT_NAMES);
  if (!attr)
    return result;
  result.reserve(attr.size());
  for (mlir::Attribute element : attr) {
    if (auto strAttr = mlir::dyn_cast<mlir::StringAttr>(element))
      result.push_back(strAttr.getValue());
  }
  return result;
}

/// Write `names` back as the `input_names` ArrayAttr on `op`.
inline void writeInputNames(mlir::Operation *op,
                            llvm::ArrayRef<llvm::StringRef> names,
                            mlir::OpBuilder &builder) {
  llvm::SmallVector<mlir::Attribute, 8> elements;
  elements.reserve(names.size());
  for (llvm::StringRef name : names)
    elements.push_back(builder.getStringAttr(name));
  op->setAttr(apxm::constants::attrs::INPUT_NAMES,
              builder.getArrayAttr(elements));
}

/// Return whether an explicit `input_roles` attribute is present on `op`.
inline bool hasInputRoles(mlir::Operation *op) {
  return op->hasAttr(kInputRolesAttrName);
}

/// Read the `input_roles` ArrayAttr off `op` into semantic role values.
///
/// Malformed entries are represented as `Invalid` so callers can preserve the
/// operation rather than silently weakening protected context semantics.
inline llvm::SmallVector<PromptInputRole>
readInputRoles(mlir::Operation *op) {
  llvm::SmallVector<PromptInputRole> result;
  auto attr = op->getAttrOfType<mlir::ArrayAttr>(kInputRolesAttrName);
  if (!attr)
    return result;
  result.reserve(attr.size());
  for (mlir::Attribute element : attr) {
    if (auto strAttr = mlir::dyn_cast<mlir::StringAttr>(element))
      result.push_back(parseInputRole(strAttr.getValue()));
    else
      result.push_back(PromptInputRole::Invalid);
  }
  return result;
}

/// Return whether `roles` is positional with the context and contains only
/// canonical AIS role values.
inline bool inputRolesAreValid(llvm::ArrayRef<PromptInputRole> roles,
                               unsigned contextSize) {
  if (roles.size() != contextSize)
    return false;
  for (PromptInputRole role : roles) {
    if (role == PromptInputRole::Invalid)
      return false;
  }
  return true;
}

/// Write `roles` back as the `input_roles` ArrayAttr on `op`.
inline void writeInputRoles(mlir::Operation *op,
                            llvm::ArrayRef<PromptInputRole> roles,
                            mlir::OpBuilder &builder) {
  llvm::SmallVector<mlir::Attribute, 8> elements;
  elements.reserve(roles.size());
  for (PromptInputRole role : roles)
    elements.push_back(builder.getStringAttr(inputRoleName(role)));
  op->setAttr(kInputRolesAttrName, builder.getArrayAttr(elements));
}

/// Build a `name -> operand-index` map from `input_names`.
inline llvm::StringMap<unsigned> nameToIndex(
    llvm::ArrayRef<llvm::StringRef> inputNames) {
  llvm::StringMap<unsigned> map;
  for (unsigned i = 0, n = inputNames.size(); i < n; ++i)
    map.insert({inputNames[i], i});
  return map;
}

/// Substitute every `{name}` in `templateStr` using `replacements`. Names
/// not present in the map are left as the literal `{name}` text. This is
/// for passes that fold compile-time constants into a template.
inline std::string substituteByName(
    llvm::StringRef templateStr,
    const llvm::StringMap<std::string> &replacements) {
  std::string result;
  result.reserve(templateStr.size());
  for (size_t i = 0, n = templateStr.size(); i < n; ) {
    if (templateStr[i] != '{') {
      result.push_back(templateStr[i]);
      ++i;
      continue;
    }
    size_t close = templateStr.find('}', i + 1);
    if (close == llvm::StringRef::npos) {
      result.append(templateStr.substr(i).str());
      break;
    }
    llvm::StringRef name = templateStr.slice(i + 1, close);
    auto it = replacements.find(name);
    if (it != replacements.end()) {
      result.append(it->second);
    } else {
      result.append(templateStr.slice(i, close + 1).str());
    }
    i = close + 1;
  }
  return result;
}

} // namespace mlir::ais::placeholders

#endif // APXM_AIS_TRANSFORMS_PLACEHOLDERS_H
