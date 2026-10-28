import protobuf from "protobufjs";

/** @param {string} message */
function reject(message) {
  throw Object.assign(new TypeError(message), { status: 400 });
}

/**
 * proto-loader coerces JSON values before the backend can validate their intent.
 * @param {protobuf.Type} type
 * @param {unknown} input
 * @returns {void}
 */
export function validateResponsibilityRequest(type, input) {
  validateMessage(type, input, type.name);
}

/** @param {protobuf.Type} type @param {unknown} input @param {string} at */
function validateMessage(type, input, at) {
  if (input === null || typeof input !== "object" || Array.isArray(input)) {
    return reject(`${at}: expected a message object`);
  }
  type.resolveAll();
  const record = /** @type {Record<string, unknown>} */ (input);
  for (const name of Object.keys(record)) {
    if (!Object.hasOwn(type.fields, name)) return reject(`${at}.${name}: unknown field`);
  }
  for (const oneof of type.oneofsArray) {
    const present = oneof.oneof.filter((name) => Object.hasOwn(record, name));
    if (present.length > 1 || (oneof.name === "selection" && present.length !== 1)) {
      return reject(`${at}.${oneof.name}: expected exactly one alternative`);
    }
  }
  for (const [name, value] of Object.entries(record)) {
    const field = type.fields[name];
    const path = `${at}.${name}`;
    if (field.repeated || field.map) return reject(`${path}: unsupported request collection`);
    if (field.resolvedType instanceof protobuf.Type) {
      validateMessage(field.resolvedType, value, path);
    } else if (field.resolvedType instanceof protobuf.Enum) {
      const values = field.resolvedType.values;
      if (
        !((typeof value === "string" && Object.hasOwn(values, value)) ||
          (typeof value === "number" && Number.isInteger(value) && Object.values(values).includes(value)))
      ) return reject(`${path}: expected a known enum name or numeric value`);
    } else if (field.type === "string") {
      if (typeof value !== "string") return reject(`${path}: expected a string`);
    } else if (field.type === "bool") {
      if (typeof value !== "boolean") return reject(`${path}: expected a boolean`);
    } else if (field.type === "uint32") {
      if (typeof value !== "number" || !Number.isInteger(value) || value < 0 || value > 4294967295) {
        return reject(`${path}: expected an exact uint32`);
      }
    } else if (field.type === "uint64") {
      if (typeof value === "number") {
        if (!Number.isSafeInteger(value) || value < 0) return reject(`${path}: expected an exact uint64`);
      } else if (typeof value !== "string" || !/^[0-9]+$/.test(value) || BigInt(value) > 18446744073709551615n) {
        return reject(`${path}: expected an exact uint64`);
      }
    } else if (field.type === "uint32") {
      if (typeof value !== "number" || !Number.isInteger(value) || value < 0 || value > 4294967295) {
        return reject(`${path}: expected an exact uint32`);
      }
    } else {
      return reject(`${path}: unsupported request scalar`);
    }
  }
}
