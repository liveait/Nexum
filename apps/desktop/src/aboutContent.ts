import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";

export type AboutVersion = { state: "loading" } | { state: "ready"; version: string } | { state: "error" };
export type ProjectLink = "website" | "source" | "documentation";
const projectLinks = new Set<ProjectLink>(["website", "source", "documentation"]);

export async function readAboutVersion(): Promise<AboutVersion> {
  try {
    const version = await getVersion();
    return typeof version === "string" && version.trim()
      ? { state: "ready", version: version.trim() }
      : { state: "error" };
  } catch {
    return { state: "error" };
  }
}

export async function openProjectLink(link: ProjectLink): Promise<void> {
  if (!projectLinks.has(link)) throw new Error("Unsupported project link");
  await invoke("open_project_link", { link });
}
