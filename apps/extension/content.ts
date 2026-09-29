// Nexum browser extension content script

let badgeRemovalTimer: number | undefined;

/** Inject "Send to Nexum" indicator on links when hovering */
function installLinkBadges(): void {
  // Add hover listener to all links
  const links = document.querySelectorAll("a[href]");
  links.forEach((link) => {
    link.addEventListener("mouseenter", () => {
      // Show small Nexum badge when hovering links that could be downloads
      const href = absoluteUrl((link as HTMLAnchorElement).getAttribute("href"));
      if (href && isDownloadUrl(href)) {
        showNexumBadge(link, href);
      }
    });
    link.addEventListener("mouseleave", () => {
      scheduleBadgeRemoval();
    });
  });
}

if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", installLinkBadges, { once: true });
} else {
  installLinkBadges();
}

/** Send a URL to the background service worker */
function requestSendToNexum(url: string): void {
  chrome.runtime?.sendMessage({ type: "sendToNexum", url });
}

/** Check if URL looks like a downloadable resource */
function isDownloadUrl(url: string): boolean {
  try {
    const parsed = new URL(url);
    return /\.(pdf|zip|tar|gz|mp4|mkv|mp3|exe|dmg|iso|torrent)$/i.test(parsed.pathname);
  } catch {
    return false;
  }
}

/** Show a small Nexum badge near a link */
function showNexumBadge(link: Element, url: string): void {
  cancelBadgeRemoval();
  removeNexumBadge();
  const badge = document.createElement("div");
  const linkRect = link.getBoundingClientRect();
  badge.id = "nexum-badge";
  badge.style.cssText = `
    position: fixed;
    left: ${Math.max(4, Math.min(linkRect.right + 8, window.innerWidth - 72))}px;
    top: ${Math.max(4, Math.min(linkRect.top, window.innerHeight - 30))}px;
    background: #1a1a2e;
    color: white;
    padding: 4px 8px;
    border-radius: 4px;
    font-size: 11px;
    z-index: 99999;
    cursor: pointer;
  `;
  badge.textContent = "Nexum";
  badge.onclick = () => requestSendToNexum(url);
  badge.addEventListener("mouseenter", cancelBadgeRemoval);
  badge.addEventListener("mouseleave", scheduleBadgeRemoval);
  document.body.appendChild(badge);
}

/** Resolve a link against the page URL before handing it to the extension. */
function absoluteUrl(href: string | null): string | null {
  if (!href) {
    return null;
  }
  try {
    const url = new URL(href, document.baseURI);
    if (url.protocol !== "http:" && url.protocol !== "https:") {
      return null;
    }
    return url.href;
  } catch {
    return null;
  }
}

/** Remove the Nexum badge if it exists */
function removeNexumBadge(): void {
  const badge = document.getElementById("nexum-badge");
  if (badge) badge.remove();
}

/** Keep the badge alive long enough for the pointer to move from the link. */
function scheduleBadgeRemoval(): void {
  cancelBadgeRemoval();
  badgeRemovalTimer = window.setTimeout(() => {
    badgeRemovalTimer = undefined;
    removeNexumBadge();
  }, 400);
}

function cancelBadgeRemoval(): void {
  if (badgeRemovalTimer !== undefined) {
    window.clearTimeout(badgeRemovalTimer);
    badgeRemovalTimer = undefined;
  }
}
