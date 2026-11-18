import { validateResponsibilityRequest } from "./responsibility-request.mjs";

/** @param {import("protobufjs").Type} type @param {unknown} input */
export function validateCatalogRequest(type, input) {
  validateResponsibilityRequest(type, input);
}
