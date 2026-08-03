import { useRef, useState, useEffect, useCallback, RefObject } from "react";
import { Virtualizer } from "@tanstack/react-virtual";

interface UseAutoScrollProps {
    scrollRef: RefObject<HTMLDivElement | null>;
    segments: any[];
    isRecording: boolean;
    isPaused: boolean;
    activeSegmentId?: string;
    virtualizer?: Virtualizer<HTMLDivElement, Element>;
    virtualizationThreshold?: number;
    disableAutoScroll?: boolean; // Completely disable auto-scroll behavior (for meeting details page)
}

interface UseAutoScrollReturn {
    autoScroll: boolean;
    setAutoScroll: (value: boolean) => void;
    scrollToBottom: () => void;
}

// Threshold in pixels to consider "at the bottom"
const SCROLL_THRESHOLD = 100;

/**
 * Custom hook to manage auto-scrolling behavior for transcript
 *
 * Features:
 * - Auto-scrolls to bottom when new content arrives during recording
 * - Pauses auto-scroll when user manually scrolls up
 * - Resumes auto-scroll when user scrolls back to the bottom
 *
 * @param segments - Array of transcript segments
 * @param isRecording - Whether recording is in progress
 * @param isPaused - Whether recording is paused
 * @param activeSegmentId - ID of the currently active segment
 * @returns Scroll ref, auto-scroll state, and scroll control functions
 */
export function useAutoScroll({
    scrollRef,
    segments,
    isRecording,
    isPaused,
    activeSegmentId,
    virtualizer,
    virtualizationThreshold = 10,
    disableAutoScroll = false,
}: UseAutoScrollProps): UseAutoScrollReturn {
    const useVirtualization = virtualizer && segments.length >= virtualizationThreshold;
    const [autoScroll, setAutoScroll] = useState(true);
    // Ref to always have current autoScroll value in effects
    const autoScrollRef = useRef(autoScroll);
    autoScrollRef.current = autoScroll;

    // Track if user has manually scrolled (to disable auto-scroll temporarily)
    const userScrolledRef = useRef(false);
    // Track if we're doing a programmatic scroll
    const isProgrammaticScrollRef = useRef(false);
    // Track previous segment count to detect new segments
    const prevSegmentCountRef = useRef(segments.length);

    /**
     * Check if the user is scrolled near the bottom
     */
    const isNearBottom = useCallback(() => {
        if (!scrollRef.current) return true;
        const { scrollTop, scrollHeight, clientHeight } = scrollRef.current;
        return scrollHeight - scrollTop - clientHeight <= SCROLL_THRESHOLD;
    }, [scrollRef]);

    /**
     * Hold the "this scroll is ours" flag until the animation actually finishes.
     *
     * A smooth scroll emits scroll events for several hundred milliseconds, and every
     * intermediate position is *not* near the bottom. If the flag were cleared early,
     * the scroll handler would read one of those positions, conclude the user had
     * scrolled away, and switch auto-scroll off mid-animation — which would stop the
     * transcript following the speaker after the very first line.
     */
    const settleProgrammaticScroll = useCallback(() => {
        const element = scrollRef.current;
        if (!element) return;

        let timeout: ReturnType<typeof setTimeout>;
        const done = () => {
            isProgrammaticScrollRef.current = false;
            element.removeEventListener("scrollend", done);
            clearTimeout(timeout);
        };

        // `scrollend` is exact where supported; the timeout is the fallback for engines
        // that do not have it.
        element.addEventListener("scrollend", done, { once: true });
        timeout = setTimeout(done, 800);
    }, [scrollRef]);

    /**
     * Scroll to bottom programmatically
     */
    const scrollToBottom = useCallback(() => {
        if (scrollRef.current) {
            isProgrammaticScrollRef.current = true;
            scrollRef.current.scrollTo({
                top: scrollRef.current.scrollHeight,
                behavior: "smooth",
            });
            userScrolledRef.current = false;
            setAutoScroll(true);
            settleProgrammaticScroll();
        }
    }, [scrollRef, settleProgrammaticScroll]);

    // Handle scroll events to detect manual scrolling
    useEffect(() => {
        const container = scrollRef.current;
        if (!container) return;

        let scrollTimeout: ReturnType<typeof setTimeout> | null = null;

        const handleScroll = () => {
            // Skip if this is a programmatic scroll
            if (isProgrammaticScrollRef.current) {
                return;
            }

            // Debounce scroll handling to prevent rapid state changes
            if (scrollTimeout) {
                clearTimeout(scrollTimeout);
            }

            scrollTimeout = setTimeout(() => {
                // Check if user is near bottom
                const nearBottom = isNearBottom();

                if (nearBottom) {
                    // User scrolled to bottom - re-enable auto-scroll
                    userScrolledRef.current = false;
                    setAutoScroll(true);
                } else {
                    // User scrolled away from bottom - disable auto-scroll
                    userScrolledRef.current = true;
                    setAutoScroll(false);
                }
            }, 100);
        };

        container.addEventListener("scroll", handleScroll, { passive: true });

        return () => {
            container.removeEventListener("scroll", handleScroll);
            if (scrollTimeout) {
                clearTimeout(scrollTimeout);
            }
        };
    }, [isNearBottom, scrollRef]);

    // Auto-scroll to bottom when new segments arrive during recording
    useEffect(() => {
        // EARLY RETURN: If auto-scroll is completely disabled (e.g., meeting details page)
        if (disableAutoScroll) {
            return;
        }

        const segmentCount = segments.length;
        const prevCount = prevSegmentCountRef.current;
        const hasNewSegments = segmentCount > prevCount;

        // Update the ref for next comparison
        prevSegmentCountRef.current = segmentCount;

        // Only scroll if new segments arrived AND user is currently at bottom
        // Check isNearBottom() immediately to avoid race conditions with the debounced scroll handler
        if (hasNewSegments && autoScrollRef.current && isRecording && !isPaused && segmentCount > 0) {
            // Check if user is at bottom RIGHT NOW before scrolling
            const isCurrentlyAtBottom = isNearBottom();
            if (!isCurrentlyAtBottom) {
                // User has scrolled up - don't auto-scroll
                return;
            }

            isProgrammaticScrollRef.current = true;

            // Animate rather than jump. This used to assign `scrollTop = scrollHeight`,
            // which teleports the list. When several lines land at once the jump is
            // proportionally larger and harsher.
            if (useVirtualization && virtualizer) {
                const totalSize = virtualizer.getTotalSize();
                virtualizer.scrollToOffset(totalSize + 1000, {
                    align: "end",
                    behavior: "smooth",
                });
            } else if (scrollRef.current) {
                scrollRef.current.scrollTo({
                    top: scrollRef.current.scrollHeight,
                    behavior: "smooth",
                });
            }

            settleProgrammaticScroll();
        }
    }, [segments.length, isRecording, isPaused, useVirtualization, virtualizer, scrollRef, isNearBottom, disableAutoScroll, settleProgrammaticScroll]);

    // Auto-scroll to active segment (when clicking on search results, etc.)
    useEffect(() => {
        if (activeSegmentId) {
            isProgrammaticScrollRef.current = true;

            if (useVirtualization && virtualizer) {
                const index = segments.findIndex((s: any) => s.id === activeSegmentId);
                if (index >= 0) {
                    virtualizer.scrollToIndex(index, { align: "center", behavior: "smooth" });
                }
            } else {
                const element = document.getElementById(`segment-${activeSegmentId}`);
                if (element) {
                    element.scrollIntoView({ behavior: "smooth", block: "center" });
                }
            }

            // Reset the flag after scroll animation completes
            setTimeout(() => {
                isProgrammaticScrollRef.current = false;
            }, 500);
        }
    }, [activeSegmentId, useVirtualization, virtualizer, segments]);

    return {
        autoScroll,
        setAutoScroll,
        scrollToBottom,
    };
}
