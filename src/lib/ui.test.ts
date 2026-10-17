import { describe, it, expect } from "vitest";
import {
  accentFg,
  recipeIcon,
  iconSrc,
  filterRecipes,
  snapIconSize,
  normalizeServiceUrl,
  customServiceDraft,
} from "./ui";

describe("accentFg", () => {
  it("returns dark text on light accents (Tauri yellow)", () => {
    expect(accentFg("#ffc131")).toBe("#1f2230");
    expect(accentFg("#ffffff")).toBe("#1f2230");
  });
  it("returns white text on dark accents", () => {
    expect(accentFg("#4f46e5")).toBe("#ffffff");
    expect(accentFg("#000000")).toBe("#ffffff");
    expect(accentFg("#16a34a")).toBe("#ffffff");
  });
  it("tolerates short / malformed hex", () => {
    expect(accentFg("#abc")).toBe("#ffffff");
    expect(accentFg("")).toBe("#ffffff");
  });
});

describe("recipeIcon / iconSrc", () => {
  it("builds the ferdium-recipes icon URL", () => {
    expect(recipeIcon("whatsapp")).toBe(
      "https://raw.githubusercontent.com/ferdium/ferdium-recipes/main/recipes/whatsapp/icon.svg",
    );
  });
  it("prefers the server custom icon when present", () => {
    expect(iconSrc({ iconUrl: "https://srv/icon/42", recipeId: "slack" })).toBe(
      "https://srv/icon/42",
    );
  });
  it("falls back to the recipe icon when no custom icon", () => {
    expect(iconSrc({ iconUrl: null, recipeId: "slack" })).toBe(recipeIcon("slack"));
    expect(iconSrc({ recipeId: "gmail" })).toBe(recipeIcon("gmail"));
  });
});

describe("filterRecipes", () => {
  const recipes = [
    { id: "whatsapp", name: "WhatsApp" },
    { id: "slack", name: "Slack" },
    { id: "telegram", name: "Telegram" },
  ];
  it("returns all, sorted by name, when the query is empty", () => {
    expect(filterRecipes(recipes, "  ").map((r) => r.id)).toEqual([
      "slack",
      "telegram",
      "whatsapp",
    ]);
  });
  it("matches name or id, case-insensitively", () => {
    expect(filterRecipes(recipes, "SLA").map((r) => r.id)).toEqual(["slack"]);
    expect(filterRecipes(recipes, "gram").map((r) => r.id)).toEqual(["telegram"]);
  });
  it("returns nothing when no recipe matches", () => {
    expect(filterRecipes(recipes, "zzz")).toEqual([]);
  });
});

describe("snapIconSize", () => {
  it("keeps valid Ferdium levels", () => {
    expect(snapIconSize(24)).toBe(24);
    expect(snapIconSize(34)).toBe(28);
  });
  it("snaps legacy values to the nearest level", () => {
    expect(snapIconSize(30)).toBe(28);
    expect(snapIconSize(20)).toBe(21);
    expect(snapIconSize(100)).toBe(28);
  });
});

describe("favicon icons", () => {
  const fav = "data:image/png;base64,AAAA";
  it("uses the favicon only when enabled and fetched", () => {
    expect(iconSrc({ recipeId: "x", useFavicon: true, iconUrl: "https://srv/i" }, fav)).toBe(fav);
    expect(iconSrc({ recipeId: "x", useFavicon: false }, fav)).toBe(recipeIcon("x"));
    expect(iconSrc({ recipeId: "x", useFavicon: true })).toBe(recipeIcon("x"));
  });
});

describe("normalizeServiceUrl", () => {
  it("adds https and keeps paths", () => {
    expect(normalizeServiceUrl(" superme.ai ")).toBe("https://superme.ai/");
    expect(normalizeServiceUrl("http://intra.example.com/app")).toBe("http://intra.example.com/app");
    expect(normalizeServiceUrl("localhost:3000")).toBe("https://localhost:3000/");
  });
  it("rejects non-addresses", () => {
    expect(normalizeServiceUrl("")).toBeNull();
    expect(normalizeServiceUrl("super me")).toBeNull();
    expect(normalizeServiceUrl("superme")).toBeNull();
    expect(normalizeServiceUrl("ftp://example.com")).toBeNull();
    expect(normalizeServiceUrl("javascript://example.com")).toBeNull();
  });
});

describe("customServiceDraft", () => {
  it("derives a name from a typed address", () => {
    expect(customServiceDraft("https://www.superme.ai/")).toEqual({
      name: "Superme",
      url: "https://www.superme.ai/",
    });
    expect(customServiceDraft("superme.ai")).toEqual({ name: "Superme", url: "https://superme.ai/" });
  });
  it("keeps a plain name and leaves the URL empty", () => {
    expect(customServiceDraft("  Superme ")).toEqual({ name: "Superme", url: "" });
  });
});
