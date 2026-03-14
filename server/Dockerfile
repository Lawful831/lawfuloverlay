FROM python:3.12-slim

# Set a non-root user for security
RUN addgroup --system overlay && adduser --system --ingroup overlay overlay

WORKDIR /app

COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

COPY bot.py .

# Switch to non-root user
USER overlay

EXPOSE 8765

CMD ["python", "-u", "bot.py"]
