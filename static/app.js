document_ready(init);

function document_ready(fn) {
    if (document.readyState !== 'loading') {
        fn();
    } else {
        document.addEventListener('DOMContentLoaded', fn);
    }
}

function init() {
    // DOM elements
    const providerSelect = document.getElementById('provider-select');
    const authTokenInput = document.getElementById('auth-token');
    const toggleTokenBtn = document.getElementById('toggle-token-visibility');
    const systemPromptInput = document.getElementById('system-prompt');
    const providersList = document.getElementById('providers-list');
    
    const activeModelTitle = document.getElementById('active-model-title');
    const activeModelDesc = document.getElementById('active-model-desc');
    const chatHistory = document.getElementById('chat-history');
    
    const uploadPreviewContainer = document.getElementById('upload-preview-container');
    const fileUploadInput = document.getElementById('file-upload');
    const chatInput = document.getElementById('chat-input');
    const btnSend = document.getElementById('btn-send');
    const chatForm = document.getElementById('chat-form');

    // App state
    let chatMessages = []; // Full history: { role, content }
    let selectedAttachments = []; // Active uploads: { name, dataUrl, type }

    const autocompleteProviders = [
        { key: '@claude', name: 'claude', display: 'Claude Code', desc: 'Anthropic Claude Code' },
        { key: '@gemini', name: 'gemini', display: 'Gemini', desc: 'Google Gemini CLI' },
        { key: '@agy', name: 'gemini', display: 'Antigravity', desc: 'Google Gemini/Agy CLI alias' },
        { key: '@openai', name: 'openai', display: 'OpenAI Codex', desc: 'OpenAI Codex' },
        { key: '@codex', name: 'openai', display: 'OpenAI Codex', desc: 'OpenAI Codex alias' }
    ];

    const mentionSuggestions = document.getElementById('mention-suggestions');
    let suggestionActive = false;
    let activeSuggestionIndex = 0;
    let filteredSuggestions = [];
    let mentionStartIndex = -1;

    const modelMetaData = {
        claude: {
            title: "Claude Code",
            desc: "Anthropic local-first CLI model with full workspace privileges"
        },
        gemini: {
            title: "Gemini / Antigravity CLI",
            desc: "Google Ultra plan model proxying via local Agy binary"
        },
        openai: {
            title: "OpenAI Codex CLI / API",
            desc: "Dual-mode OpenAI model routing via local Codex binary or direct REST fallback"
        }
    };

    // Show/hide bearer token
    toggleTokenBtn.addEventListener('click', () => {
        if (authTokenInput.type === 'password') {
            authTokenInput.type = 'text';
            toggleTokenBtn.textContent = '🔒';
        } else {
            authTokenInput.type = 'password';
            toggleTokenBtn.textContent = '👁️';
        }
    });

    // Handle provider selection change
    providerSelect.addEventListener('change', () => {
        const val = providerSelect.value;
        if (modelMetaData[val]) {
            activeModelTitle.textContent = modelMetaData[val].title;
            activeModelDesc.textContent = modelMetaData[val].desc;
        }
    });

    // Check host CLI environment status
    async function checkCliEnvironment() {
        const token = authTokenInput.value.trim();

        try {
            const resp = await fetch('/api/providers', {
                method: 'GET',
                headers: {
                    'Authorization': `Bearer ${token}`
                }
            });
            
            if (!resp.ok) {
                providersList.innerHTML = `<div class="provider-status-row" style="color: var(--accent-red); border-color: rgba(239,68,68,0.2);">
                    Failed to query providers list (HTTP ${resp.status})
                </div>`;
                return;
            }

            const data = await resp.json();
            providersList.innerHTML = '';
            
            data.forEach(provider => {
                const row = document.createElement('div');
                row.className = 'provider-status-row';
                
                const displayTitle = provider.name === 'claude' ? 'Claude Code' : 
                                     provider.name === 'gemini' ? 'Agy (Antigravity)' : 'Codex (OpenAI)';
                
                row.innerHTML = `
                    <div class="provider-info">
                        <span class="provider-name">${displayTitle}</span>
                        <span class="provider-vision">${provider.supports_vision ? 'Vision + Text' : 'Text Only'}</span>
                    </div>
                    <span class="status-badge ${provider.available ? 'available' : 'unavailable'}">
                        ${provider.available ? 'Available' : 'Unavailable'}
                    </span>
                `;
                providersList.appendChild(row);
            });
        } catch (e) {
            providersList.innerHTML = `<div class="provider-status-row" style="color: var(--accent-red); border-color: rgba(239,68,68,0.2);">
                Server connection error. Is gateway running?
            </div>`;
        }
    }

    // Call environment probe immediately, and poll every 15s
    checkCliEnvironment();
    setInterval(checkCliEnvironment, 15000);
    authTokenInput.addEventListener('change', checkCliEnvironment);

    // Auto-expand textarea height up to a limit
    chatInput.addEventListener('input', () => {
        chatInput.style.height = 'auto';
        chatInput.style.height = (chatInput.scrollHeight) + 'px';
        handleMentionInput();
    });

    chatInput.addEventListener('keydown', (e) => {
        if (suggestionActive) {
            if (e.key === 'ArrowDown') {
                e.preventDefault();
                activeSuggestionIndex = (activeSuggestionIndex + 1) % filteredSuggestions.length;
                renderSuggestions();
            } else if (e.key === 'ArrowUp') {
                e.preventDefault();
                activeSuggestionIndex = (activeSuggestionIndex - 1 + filteredSuggestions.length) % filteredSuggestions.length;
                renderSuggestions();
            } else if (e.key === 'Enter' || e.key === 'Tab') {
                e.preventDefault();
                if (filteredSuggestions[activeSuggestionIndex]) {
                    insertSuggestion(filteredSuggestions[activeSuggestionIndex].key);
                }
            } else if (e.key === 'Escape') {
                e.preventDefault();
                hideSuggestions();
            }
        }
    });

    function handleMentionInput() {
        const text = chatInput.value;
        const cursor = chatInput.selectionStart;
        
        const lastAt = text.lastIndexOf('@', cursor - 1);
        if (lastAt === -1) {
            hideSuggestions();
            return;
        }
        
        const textBetween = text.substring(lastAt + 1, cursor);
        if (/\s/.test(textBetween)) {
            hideSuggestions();
            return;
        }
        
        const query = textBetween.toLowerCase();
        const matches = autocompleteProviders.filter(p => p.key.startsWith('@' + query));
        
        if (matches.length > 0) {
            showSuggestions(matches, lastAt);
        } else {
            hideSuggestions();
        }
    }

    function showSuggestions(list, startIndex) {
        filteredSuggestions = list;
        mentionStartIndex = startIndex;
        suggestionActive = true;
        activeSuggestionIndex = 0;
        
        renderSuggestions();
        mentionSuggestions.classList.remove('hidden');
    }

    function hideSuggestions() {
        suggestionActive = false;
        filteredSuggestions = [];
        mentionStartIndex = -1;
        mentionSuggestions.classList.add('hidden');
        mentionSuggestions.innerHTML = '';
    }

    function renderSuggestions() {
        mentionSuggestions.innerHTML = '';
        filteredSuggestions.forEach((item, index) => {
            const div = document.createElement('div');
            div.className = `suggestion-item ${item.name} ${index === activeSuggestionIndex ? 'active' : ''}`;
            div.innerHTML = `
                <span class="suggestion-key">${item.key}</span>
                <span class="suggestion-desc">${item.desc}</span>
            `;
            
            div.addEventListener('click', () => {
                insertSuggestion(item.key);
            });
            mentionSuggestions.appendChild(div);
        });
    }

    function insertSuggestion(key) {
        const text = chatInput.value;
        const before = text.substring(0, mentionStartIndex);
        const after = text.substring(chatInput.selectionStart);
        
        chatInput.value = before + key + ' ' + after;
        chatInput.focus();
        
        const newCursorPos = mentionStartIndex + key.length + 1;
        chatInput.setSelectionRange(newCursorPos, newCursorPos);
        
        hideSuggestions();
        
        chatInput.style.height = 'auto';
        chatInput.style.height = (chatInput.scrollHeight) + 'px';
    }

    // Handle file uploads selection
    fileUploadInput.addEventListener('change', () => {
        const files = Array.from(fileUploadInput.files);
        
        files.forEach(file => {
            if (selectedAttachments.length >= 10) {
                alert("You can attach a maximum of 10 files.");
                return;
            }

            const reader = new FileReader();
            reader.onload = (e) => {
                const dataUrl = e.target.result;
                const type = file.type;
                
                const attachmentObj = {
                    name: file.name,
                    dataUrl: dataUrl,
                    type: type
                };
                
                selectedAttachments.push(attachmentObj);
                renderPreviewChips();
            };
            reader.readAsDataURL(file);
        });
        
        // Reset file input value so same file can be selected again
        fileUploadInput.value = '';
    });

    // Render file preview chips
    function renderPreviewChips() {
        if (selectedAttachments.length === 0) {
            uploadPreviewContainer.classList.add('hidden');
            return;
        }

        uploadPreviewContainer.classList.remove('hidden');
        uploadPreviewContainer.innerHTML = '';

        selectedAttachments.forEach((file, index) => {
            const chip = document.createElement('div');
            chip.className = 'preview-chip';
            
            const isImage = file.type.startsWith('image/');
            
            if (isImage) {
                chip.innerHTML = `
                    <img src="${file.dataUrl}" class="preview-thumbnail" alt="thumbnail">
                    <span class="preview-filename">${file.name}</span>
                    <button type="button" class="btn-remove-preview" data-index="${index}">×</button>
                `;
            } else {
                chip.innerHTML = `
                    <span class="preview-file-icon">📄</span>
                    <span class="preview-filename">${file.name}</span>
                    <button type="button" class="btn-remove-preview" data-index="${index}">×</button>
                `;
            }
            
            uploadPreviewContainer.appendChild(chip);
        });

        // Add delete event listeners
        document.querySelectorAll('.btn-remove-preview').forEach(btn => {
            btn.addEventListener('click', (e) => {
                const idx = parseInt(e.target.getAttribute('data-index'));
                selectedAttachments.splice(idx, 1);
                renderPreviewChips();
            });
        });
    }

    // Submit prompt
    chatForm.addEventListener('submit', async (e) => {
        e.preventDefault();
        
        const rawTextPrompt = chatInput.value.trim();
        if (!rawTextPrompt && selectedAttachments.length === 0) return;

        hideSuggestions();
        
        let textPrompt = rawTextPrompt;
        let selectedModel = providerSelect.value;
        let mentionModel = null;

        const mentionRegex = /^@(claude|gemini|agy|openai|codex)\b/i;
        const match = textPrompt.match(mentionRegex);
        if (match) {
            const mentionTag = match[1].toLowerCase();
            if (mentionTag === 'agy') {
                selectedModel = 'gemini';
                mentionModel = 'gemini';
            } else if (mentionTag === 'codex') {
                selectedModel = 'openai';
                mentionModel = 'openai';
            } else {
                selectedModel = mentionTag;
                mentionModel = mentionTag;
            }
            textPrompt = textPrompt.replace(mentionRegex, '').trim();
        }

        // Display user message bubble with mention highlighting
        appendMessageBubble('user', rawTextPrompt, selectedAttachments, mentionModel);
        
        // Disable form inputs
        setLoadingState(true);
        
        const token = authTokenInput.value.trim();
        const systemPrompt = systemPromptInput.value.trim();

        // Compile payload
        const messagesPayload = [];
        
        if (systemPrompt) {
            messagesPayload.push({
                role: 'system',
                content: systemPrompt
            });
        }
        
        chatMessages.forEach(msg => {
            messagesPayload.push(msg);
        });

        let userContent;
        if (selectedAttachments.length === 0) {
            userContent = textPrompt;
        } else {
            userContent = [];
            if (textPrompt) {
                userContent.push({
                    type: 'text',
                    text: textPrompt
                });
            }
            selectedAttachments.forEach(att => {
                if (att.type.startsWith('image/')) {
                    userContent.push({
                        type: 'image_url',
                        image_url: {
                            url: att.dataUrl
                        }
                    });
                } else {
                    userContent.push({
                        type: 'text',
                        text: `\n\n[Attached file: ${att.name}]\n${att.dataUrl}`
                    });
                }
            });
        }

        messagesPayload.push({
            role: 'user',
            content: userContent
        });

        const requestBody = {
            model: selectedModel,
            messages: messagesPayload,
            timeout: 120
        };

        // Reset inputs
        chatInput.value = '';
        chatInput.style.height = 'auto';
        const attachmentsCopy = [...selectedAttachments];
        selectedAttachments = [];
        renderPreviewChips();

        try {
            const response = await fetch('/v1/chat/completions', {
                method: 'POST',
                headers: {
                    'Content-Type': 'application/json',
                    'Authorization': `Bearer ${token}`
                },
                body: JSON.stringify(requestBody)
            });

            if (!response.ok) {
                const errData = await response.json().catch(() => ({}));
                const errMsg = errData.error?.message || `HTTP ${response.status} Error`;
                appendMessageBubble('system-msg', `Error: ${errMsg}`);
                setLoadingState(false);
                return;
            }

            const data = await response.json();
            const reply = data.choices[0].message.content;

            chatMessages.push({
                role: 'user',
                content: textPrompt
            });
            chatMessages.push({
                role: 'assistant',
                content: reply
            });

            // Display AI bubble
            appendMessageBubble('ai', reply, [], selectedModel);
            
        } catch (error) {
            appendMessageBubble('system-msg', `Connection error: ${error.message}`);
        } finally {
            setLoadingState(false);
        }
    });

    function setLoadingState(loading) {
        if (loading) {
            btnSend.disabled = true;
            chatInput.disabled = true;
            document.querySelector('.send-text').classList.add('hidden');
            document.querySelector('.send-spinner').classList.remove('hidden');
        } else {
            btnSend.disabled = false;
            chatInput.disabled = false;
            document.querySelector('.send-text').classList.remove('hidden');
            document.querySelector('.send-spinner').classList.add('hidden');
            chatInput.focus();
        }
    }

    function appendMessageBubble(role, text, attachments = [], modelOverride = null) {
        const msgDiv = document.createElement('div');
        msgDiv.className = `message ${role}`;
        
        if (role === 'system-msg') {
            msgDiv.innerHTML = `<div class="msg-content">${text}</div>`;
            chatHistory.appendChild(msgDiv);
            chatHistory.scrollTop = chatHistory.scrollHeight;
            return;
        }

        const activeModel = modelOverride || providerSelect.value;
        const senderLabel = role === 'user' ? 'You' : 
                            activeModel === 'claude' ? 'Claude' :
                            activeModel === 'gemini' ? 'Antigravity' : 'Codex';
        
        let contentHtml = '';
        if (role === 'user') {
            const mentionRegex = /^@(claude|gemini|agy|openai|codex)\b/i;
            const match = text.match(mentionRegex);
            if (match) {
                const mentionText = match[0];
                const mentionClass = match[1].toLowerCase();
                const remainingText = text.replace(mentionRegex, '').trim();
                
                contentHtml = `<span class="mention-badge ${mentionClass}">${escapeHtml(mentionText)}</span> ` + escapeHtml(remainingText);
            } else {
                contentHtml = escapeHtml(text);
            }

            if (attachments.length > 0) {
                contentHtml += '<div class="msg-attachments">';
                attachments.forEach(att => {
                    if (att.type.startsWith('image/')) {
                        contentHtml += `<img src="${att.dataUrl}" style="max-width: 150px; border-radius: 6px; margin-top: 0.5rem; display: block;" alt="upload">`;
                    } else {
                        contentHtml += `<div style="font-size: 0.75rem; color: var(--text-secondary); margin-top: 0.25rem;">📄 ${escapeHtml(att.name)}</div>`;
                    }
                });
                contentHtml += '</div>';
            }
        } else {
            // Render markdown code fences & generated outputs from AI
            contentHtml = renderMarkdown(text);
        }

        msgDiv.innerHTML = `
            <span class="msg-sender">${senderLabel}</span>
            <div class="msg-content">${contentHtml}</div>
        `;
        
        chatHistory.appendChild(msgDiv);
        chatHistory.scrollTop = chatHistory.scrollHeight;
    }

    function escapeHtml(str) {
        if (!str) return '';
        return str.replace(/&/g, "&amp;")
                  .replace(/</g, "&lt;")
                  .replace(/>/g, "&gt;")
                  .replace(/"/g, "&quot;")
                  .replace(/'/g, "&#039;");
    }

    function renderMarkdown(text) {
        if (!text) return '';
        
        let html = escapeHtml(text);
        
        // Replace code blocks: ```lang code ```
        html = html.replace(/```(?:[a-zA-Z0-9]+)?\n([\s\S]*?)```/g, (match, code) => {
            return `<pre><code>${code}</code></pre>`;
        });
        
        // Replace inline code: `code`
        html = html.replace(/`([^`]+)`/g, '<code>$1</code>');

        // Replace output image tags: ![Generated Output](/outputs/filename.png)
        html = html.replace(/!\[Generated Output\]\(\/outputs\/([a-zA-Z0-9_\-\.]+)\)/g, (match, filename) => {
            return `<div class="outputs-header">Generated Output:</div><img src="/outputs/${filename}" class="output-image" alt="Output File">`;
        });

        // Replace output download file links: - [Download Generated File](/outputs/filename.pdf)
        html = html.replace(/\- \[Download Generated File\]\(\/outputs\/([a-zA-Z0-9_\-\.]+)\)/g, (match, filename) => {
            return `<div style="margin-top: 0.5rem;">📄 <a href="/outputs/${filename}" target="_blank" style="color: var(--accent-cyan); text-decoration: underline;">Download Generated File (${filename})</a></div>`;
        });
        
        return html;
    }
}
