/**
 * mdbook-exercises JavaScript
 *
 * Interactive functionality for exercise blocks in mdBook.
 */

(function() {
    'use strict';

    // ============================================ 
    // Configuration
    // ============================================ 
    const PLAYGROUND_URL = 'https://play.rust-lang.org';
    const STORAGE_KEY = 'mdbook-exercises-progress';

    // ============================================ 
    // Utility Functions
    // ============================================ 

    /**
     * Get exercise progress from localStorage.
     */
    function getProgress() {
        try {
            const stored = localStorage.getItem(STORAGE_KEY);
            return stored ? JSON.parse(stored) : {};
        } catch (e) {
            console.warn('Failed to load exercise progress:', e);
            return {};
        }
    }

    /**
     * Save exercise progress to localStorage.
     */
    function saveProgress(progress) {
        try {
            localStorage.setItem(STORAGE_KEY, JSON.stringify(progress));
        } catch (e) {
            console.warn('Failed to save exercise progress:', e);
        }
    }

    /**
     * Get the exercise ID from a container element.
     */
    function getExerciseId(container) {
        const idEl = container.querySelector('.exercise-id');
        if (idEl) return idEl.textContent.trim();
        // Try data attribute on the article itself
        if (container.dataset && container.dataset.exerciseId) {
            return container.dataset.exerciseId;
        }
        // Try looking for article child
        const article = container.querySelector('.exercise, .usecase-exercise');
        if (article && article.dataset && article.dataset.exerciseId) {
            return article.dataset.exerciseId;
        }
        return null;
    }

    /**
     * Show a temporary notification.
     */
    function showNotification(message, type) {
        const notification = document.createElement('div');
        notification.className = `exercise-notification ${type}`;
        notification.textContent = message;
        notification.style.cssText = "
            position: fixed;
            bottom: 20px;
            right: 20px;
            padding: 12px 20px;
            border-radius: 4px;
            font-size: 14px;
            z-index: 10000;
            animation: slideIn 0.3s ease;
        ";

        if (type === 'success') {
            notification.style.background = '#d4edda';
            notification.style.color = '#155724';
            notification.style.border = '1px solid #c3e6cb';
        } else if (type === 'error') {
            notification.style.background = '#f8d7da';
            notification.style.color = '#721c24';
            notification.style.border = '1px solid #f5c6cb';
        } else {
            notification.style.background = '#fff3cd';
            notification.style.color = '#856404';
            notification.style.border = '1px solid #ffeeba';
        }

        document.body.appendChild(notification);

        setTimeout(() => {
            notification.style.animation = 'slideOut 0.3s ease';
            setTimeout(() => notification.remove(), 300);
        }, 2000);
    }

    // ============================================ 
    // Copy Button
    // ============================================ 

    function initCopyButtons() {
        document.querySelectorAll('.btn-copy').forEach(button => {
            button.addEventListener('click', async function() {
                const targetId = this.dataset.target;
                const textarea = document.getElementById(targetId);

                if (!textarea) return;

                try {
                    await navigator.clipboard.writeText(textarea.value);
                    const originalText = this.textContent;
                    this.textContent = 'Copied!';
                    setTimeout(() => {
                        this.textContent = originalText;
                    }, 1500);
                } catch (e) {
                    console.error('Failed to copy:', e);
                    showNotification('Failed to copy code', 'error');
                }
            });
        });
    }

    // ============================================ 
    // Reset Button
    // ============================================ 

    function initResetButtons() {
        document.querySelectorAll('.btn-reset').forEach(button => {
            button.addEventListener('click', function() {
                const targetId = this.dataset.target;
                const textarea = document.getElementById(targetId);

                if (textarea && textarea.dataset.original) {
                    if (confirm('Reset code to original? Your changes will be lost.')) {
                        // Decode HTML entities from data-original
                        const temp = document.createElement('textarea');
                        temp.innerHTML = textarea.dataset.original;
                        textarea.value = temp.value;
                        showNotification('Code reset to original', 'success');
                    }
                } else {
                    showNotification('No original code found', 'error');
                }
            });
        });
    }

    // ============================================ 
    // Hint Toggles
    // ============================================ 

    function initSolutionToggles() {
        document.querySelectorAll('.solution-toggle').forEach(toggle => {
            // Initialize label and aria-expanded from current state
            const details = toggle.closest('details');
            if (details) {
                toggle.setAttribute('aria-expanded', details.open ? 'true' : 'false');
                toggle.textContent = details.open ? 'Hide Solution' : 'Show Solution';
            }

            toggle.addEventListener('click', function(e) {
                e.preventDefault();
                const details = this.closest('details');
                if (!details) return;

                if (!this.dataset.confirmed) {
                    if (!confirm('Are you sure you want to reveal the solution? Try the hints first!')) {
                        return;
                    }
                    this.dataset.confirmed = 'true';
                }

                details.open = !details.open;
                const open = details.open;
                this.setAttribute('aria-expanded', open ? 'true' : 'false');
                this.textContent = open ? 'Hide Solution' : 'Show Solution';
            });
        });
    }

    // ============================================ 
    // Progress Tracking
    // ============================================ 

    function initProgressTracking() {
        const progress = getProgress();

        document.querySelectorAll('.exercise-container, .exercise, .usecase-exercise').forEach(container => {
            const exerciseId = getExerciseId(container);
            if (!exerciseId) return;

            const article = container.classList.contains('exercise') || container.classList.contains('usecase-exercise') 
                ? container 
                : container.querySelector('.exercise, .usecase-exercise');
                
            const completeBtn = container.querySelector('.btn-complete');
            const checkbox = container.querySelector('.progress-indicator input[type="checkbox"]');

            const markCompletedUI = (completed) => {
                if (checkbox) {
                    checkbox.checked = completed;
                    checkbox.closest('.progress-indicator')?.classList.toggle('completed', completed);
                }
                if (article) {
                    article.classList.toggle('completed', completed);
                }
            };

            // Restore saved state
            if (progress[exerciseId]?.completed) {
                markCompletedUI(true);
            }

            // Handle complete button
            if (completeBtn) {
                completeBtn.addEventListener('click', function() {
                    const current = getProgress();
                    const already = !!current[exerciseId]?.completed;
                    if (already) {
                        delete current[exerciseId];
                        markCompletedUI(false);
                    } else {
                        current[exerciseId] = { completed: true, timestamp: new Date().toISOString() };
                        markCompletedUI(true);
                        showNotification('Exercise marked as complete!', 'success');
                    }
                    saveProgress(current);
                });
            }

            // Handle checkbox if present
            if (checkbox) {
                checkbox.addEventListener('change', function() {
                    const current = getProgress();
                    if (this.checked) {
                        current[exerciseId] = { completed: true, timestamp: new Date().toISOString() };
                        markCompletedUI(true);
                        showNotification('Exercise marked as complete!', 'success');
                    } else {
                        delete current[exerciseId];
                        markCompletedUI(false);
                    }
                    saveProgress(current);
                });
            }
        });
    }

    // ============================================ 
    // Run Tests (Rust Playground Integration)
    // ============================================ 

    function initRunTests() {
        document.querySelectorAll('.btn-run-tests').forEach(button => {
            button.addEventListener('click', async function() {
                const container = this.closest('.exercise-container, .exercise');
                const starterContainer = container.querySelector('.exercise-starter');
                const testsContainer = container.querySelector('.exercise-tests');

                // Get the user's code
                let userCode = '';
                const textarea = starterContainer?.querySelector('textarea');
                if (textarea) {
                    userCode = textarea.value;
                }

                // Get the test code
                const testCodeEl = testsContainer?.querySelector('pre code');
                const testCode = testCodeEl ? testCodeEl.textContent : '';

                if (!userCode && !testCode) {
                    showNotification('No code to run', 'error');
                    return;
                }

                // Combine user code and test code
                const fullCode = combineCodeForTests(userCode, testCode);

                // Show loading state
                this.classList.add('loading');
                this.disabled = true;
                this.textContent = 'Running...';

                const resultsEl = testsContainer?.querySelector('.test-results');
                if (resultsEl) {
                    resultsEl.classList.remove('success', 'error', 'pending', 'show');
                    resultsEl.classList.add('pending', 'show');
                    resultsEl.textContent = 'Compiling and running tests...';
                }

                try {
                    const result = await runOnPlayground(fullCode);

                    if (resultsEl) {
                        resultsEl.classList.remove('pending');

                        if (result.success) {
                            resultsEl.classList.add('success');
                            resultsEl.textContent = result.output || 'All tests passed!';
                        } else {
                            resultsEl.classList.add('error');
                            resultsEl.textContent = result.output || result.error || 'Tests failed';
                        }
                    }

                    if (result.success) {
                        showNotification('Tests passed!', 'success');
                    } else {
                        showNotification('Tests failed', 'error');
                    }

                } catch (e) {
                    console.error('Failed to run tests:', e);
                    if (resultsEl) {
                        resultsEl.classList.remove('pending');
                        resultsEl.classList.add('error');
                        resultsEl.textContent = 'Failed to connect to Rust Playground: ' + e.message;
                    }
                    showNotification('Failed to run tests', 'error');
                } finally {
                    this.classList.remove('loading');
                    this.disabled = false;
                    this.textContent = 'Run Tests';
                }
            });
        });
    }

    /**
     * Combine user code and test code for Playground execution.
     */
    function combineCodeForTests(userCode, testCode) {
        // If the test code includes #[cfg(test)], we need to handle it specially
        // Otherwise, just combine them

        // Remove any existing main function from user code if tests have their own
        let combined = userCode;

        if (testCode) {
            // Check if user code has a main function
            const hasMain = /fn\s+main\s*\(/.test(userCode);
            const testHasMain = /fn\s+main\s*\(/.test(testCode);

            if (hasMain && testHasMain) {
                // Remove main from test code if user code has one
                combined = userCode + '\n\n' + testCode.replace(/fn\s+main\s*\([^)]*\)\s*\{[^}]*\}/g, '');
            } else {
                combined = userCode + '\n\n' + testCode;
            }
        }

        return combined;
    }

    /**
     * Run code on the Rust Playground.
     */
    async function runOnPlayground(code) {
        const response = await fetch(PLAYGROUND_URL + '/execute', {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json',
            },
            body: JSON.stringify({
                channel: 'stable',
                mode: 'debug',
                edition: '2021',
                crateType: 'lib',
                tests: true,
                code: code,
                backtrace: false
            })
        });

        if (!response.ok) {
            throw new Error(`Playground returned ${response.status}`);
        }

        const result = await response.json();

        return {
            success: result.success,
            output: result.stdout + result.stderr,
            error: result.stderr
        };
    }

    // ============================================ 
    // UseCase Exercise Word Count
    // ============================================ 

    function initWordCount() {
        document.querySelectorAll('.usecase-exercise .response-editor').forEach(textarea => {
            const meta = textarea.closest('.exercise-response').querySelector('.current-word-count');
            const submitBtn = textarea.closest('.exercise-response').querySelector('.btn-submit');
            const minWords = parseInt(textarea.dataset.minWords || 0, 10);
            const maxWords = parseInt(textarea.dataset.maxWords || 10000, 10);

            const updateCount = () => {
                const text = textarea.value.trim();
                const wordCount = text ? text.split(/\s+/).length : 0;
                if (meta) meta.textContent = `${wordCount} words`;

                if (wordCount < minWords || wordCount > maxWords) {
                    if (meta) meta.style.color = '#dc3545';
                    if (submitBtn) submitBtn.disabled = true;
                } else {
                    if (meta) meta.style.color = '#28a745';
                    if (submitBtn) submitBtn.disabled = false;
                }
            };

            textarea.addEventListener('input', updateCount);
            // Initial check
            updateCount();
        });
    }

    // ============================================ 
    // UseCase Exercise Submission (Mock)
    // ============================================ 

    function initUseCaseSubmission() {
        document.querySelectorAll('.usecase-exercise .btn-submit').forEach(btn => {
            btn.addEventListener('click', async function() {
                const container = this.closest('.usecase-exercise');
                const textarea = container.querySelector('.response-editor');
                const evaluationSection = container.querySelector('.exercise-evaluation');
                const contextSection = container.querySelector('.exercise-context');

                if (!textarea.value.trim()) return;

                this.disabled = true;
                this.textContent = 'Evaluating...';

                // Simulate network delay
                await new Promise(r => setTimeout(r, 1500));

                // Mock Evaluation Result
                const mockResult = {
                    score: 0.85,
                    feedback: "Good analysis. You covered the key points about HIPAA and authentication."
                };

                // Show Evaluation
                if (evaluationSection) {
                    evaluationSection.hidden = false;
                    const scoreEl = evaluationSection.querySelector('.overall-score');
                    if (scoreEl) scoreEl.textContent = `Score: ${(mockResult.score * 100).toFixed(0)}%`;
                    
                    const feedbackEl = evaluationSection.querySelector('.feedback');
                    if (feedbackEl) feedbackEl.textContent = mockResult.feedback;
                }

                // Show Context
                if (contextSection) {
                    contextSection.hidden = false;
                }

                this.textContent = 'Submitted';
                showNotification('Response submitted successfully', 'success');
            });
        });
    }

    // ============================================ 
    // Editable Code (make code blocks editable)
    // ============================================ 

    function initEditableCode() {
        document.querySelectorAll('.exercise-starter[data-editable="true"]').forEach(container => {
            const codeBlock = container.querySelector('.starter-code pre');
            const codeEl = container.querySelector('.starter-code code');

            if (!codeBlock || !codeEl) return;

            // Create textarea for editing
            const textarea = document.createElement('textarea');
            textarea.value = codeEl.textContent;
            textarea.className = 'code-editor';
            textarea.spellcheck = false;

            // Store original code for reset
            container.dataset.originalCode = codeEl.textContent;

            // Replace code block with textarea
            codeBlock.style.display = 'none';
            codeBlock.insertAdjacentElement('afterend', textarea);

            // Auto-resize textarea
            textarea.addEventListener('input', function() {
                this.style.height = 'auto';
                this.style.height = this.scrollHeight + 'px';
            });

            // Initial resize
            textarea.style.height = textarea.scrollHeight + 'px';
        });
    }

    // ============================================ 
    // Keyboard Shortcuts
    // ============================================ 

    function initKeyboardShortcuts() {
        document.addEventListener('keydown', function(e) {
            // Ctrl/Cmd + Enter to run tests
            if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') {
                const activeElement = document.activeElement;
                if (activeElement) {
                    const container = activeElement.closest('.exercise-container, .exercise');
                    if (!container) return;
                    const runBtn = container.querySelector('.btn-run-tests, .run-tests-btn');
                    if (runBtn && !runBtn.disabled) {
                        e.preventDefault();
                        runBtn.click();
                    }
                }
            }
        });
    }

    // ============================================ 
    // Add CSS Animation Styles
    // ============================================ 

    function addAnimationStyles() {
        const style = document.createElement('style');
        style.textContent = "
            @keyframes slideIn {
                from {
                    transform: translateX(100%);
                    opacity: 0;
                }
                to {
                    transform: translateX(0);
                    opacity: 1;
                }
            }
            @keyframes slideOut {
                from {
                    transform: translateX(0);
                    opacity: 1;
                }
                to {
                    transform: translateX(100%);
                    opacity: 0;
                }
            }
        ";
        document.head.appendChild(style);
    }

    // ============================================ 
    // Textarea Content Initialization
    // ============================================ 

    /**
     * Populate textareas from their data-original attribute.
     */
    function initTextareaContent() {
        document.querySelectorAll('textarea.code-editor[data-original]').forEach(textarea => {
            // Decode HTML entities from data-original
            const original = textarea.dataset.original;
            if (original) {
                // Create a temporary element to decode HTML entities
                const temp = document.createElement('textarea');
                temp.innerHTML = original;
                textarea.value = temp.value;
            }
        });
    }

    // ============================================ 
    // Section Navigation Highlighting
    // ============================================ 

    function initNavHighlighting() {
        const navLinks = document.querySelectorAll('.exercise-nav a');
        if (navLinks.length === 0) return;

        const sections = [];
        navLinks.forEach(link => {
            const href = link.getAttribute('href');
            if (href && href.startsWith('#')) {
                const section = document.getElementById(href.slice(1));
                if (section) {
                    sections.push({ element: section, link: link });
                }
            }
        });

        if (sections.length === 0) return;

        function updateActiveNav() {
            const scrollPos = window.scrollY + 100; // Offset for header

            let activeSection = sections[0];
            for (const section of sections) {
                if (section.element.offsetTop <= scrollPos) {
                    activeSection = section;
                }
            }

            navLinks.forEach(link => link.classList.remove('active'));
            if (activeSection) {
                activeSection.link.classList.add('active');
            }
        }

        // Throttle scroll events
        let ticking = false;
        window.addEventListener('scroll', () => {
            if (!ticking) {
                window.requestAnimationFrame(() => {
                    updateActiveNav();
                    ticking = false;
                });
                ticking = true;
            }
        });

        // Initial highlight
        updateActiveNav();
    }

    // ============================================ 
    // Initialize
    // ============================================ 

    function init() {
        addAnimationStyles();
        initTextareaContent();
        initCopyButtons();
        initResetButtons();
        initSolutionToggles();
        initProgressTracking();
        initRunTests();
        initEditableCode();
        initKeyboardShortcuts();
        initNavHighlighting();
        initWordCount(); // New
        initUseCaseSubmission(); // New
        applySyntaxHighlighting();

        console.log('mdbook-exercises initialized');
    }

    // Run on DOM ready
    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', init);
    } else {
        init();
    }

    // Export for testing
    if (typeof window !== 'undefined') {
        window.mdbookExercises = {
            getProgress,
            saveProgress,
            runOnPlayground
        };
    }
})();

// Apply syntax highlighting if highlight.js is available (as in mdBook)
function applySyntaxHighlighting() {
    if (typeof window === 'undefined') return;
    const hljs = window.hljs;
    if (!hljs || typeof hljs.highlightElement !== 'function') return;
    document.querySelectorAll('.exercise pre code').forEach((block) => {
        try { hljs.highlightElement(block); } catch (_) { /* ignore */ }
    });
}